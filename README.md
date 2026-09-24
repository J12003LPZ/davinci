# DaVinci

[![CI](https://github.com/J12003LPZ/davinci/actions/workflows/ci.yml/badge.svg)](https://github.com/J12003LPZ/davinci/actions/workflows/ci.yml)
[![Workflow lint](https://github.com/J12003LPZ/davinci/actions/workflows/workflow-lint.yml/badge.svg)](https://github.com/J12003LPZ/davinci/actions/workflows/workflow-lint.yml)
[![Security SARIF](https://github.com/J12003LPZ/davinci/actions/workflows/security-sarif.yml/badge.svg)](https://github.com/J12003LPZ/davinci/actions/workflows/security-sarif.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

**DaVinci is a native Rust AI coding-agent harness for the terminal.**

> **Terminal rebuild branch:** see the [implementation and review guide](docs/ui/terminal-rebuild.md) for the new shell, settings/model selectors, optional graph input, rendered previews and verification limitations.

It combines an interactive coding assistant, multi-provider model runtime, permission system, persistent sessions, engineering intelligence, multi-agent orchestration, deterministic verification, security analysis, memory and learning, MCP, extensions, and optional local voice input in one CLI.

DaVinci began as a Rust-compatible rewrite of the TypeScript [pi](https://github.com/earendil-works/pi) coding agent and has grown into a larger native harness. The pinned TypeScript source under [vendor/davinci](vendor/davinci) remains a behavioral compatibility reference. The active product is the Rust workspace in this repository.

> **Workspace version:** 1.0.71
> **Rust toolchain:** 1.83.0  
> **Primary executable:** davinci

---

## Contents

- [What DaVinci does](#what-davinci-does)
- [Installation](#installation)
- [Quick start](#quick-start)
- [Models and authentication](#models-and-authentication)
- [Permissions and project trust](#permissions-and-project-trust)
- [Execution modes](#execution-modes)
- [Major capabilities](#major-capabilities)
- [Sessions and persistence](#sessions-and-persistence)
- [Configuration](#configuration)
- [Extensions and MCP](#extensions-and-mcp)
- [Local voice input](#local-voice-input)
- [Architecture](#architecture)
- [Repository layout](#repository-layout)
- [Development](#development)
- [Documentation](#documentation)
- [Compatibility](#compatibility)
- [Security notes](#security-notes)
- [License](#license)

---

## What DaVinci does

DaVinci is the runtime around an AI coding model, not just a chat interface. It gives a model a controlled view of a repository and tools for understanding, changing, testing, and verifying code.

At a high level, DaVinci can:

- run as an interactive terminal coding assistant or non-interactive CLI;
- work with multiple model providers and reasoning levels;
- read, search, edit, write, and execute commands in a repository;
- require approvals or enforce planning/read-only policies before mutations;
- preserve resumable and branchable session history;
- inspect repository structure, packages, build systems, Git state, language semantics, tests, and change impact;
- use subagents, persistent agent teams, workflows, or a deterministic engineering graph for larger tasks;
- manage long-running processes and isolated worktrees;
- perform transactional edits with verification evidence;
- bound context growth through compaction, the Token Governor, artifact storage, and an optional Context VM;
- keep local repository-scoped memory and distill successful procedures into reusable skills;
- run security analysis and produce auditable JSON, Markdown, and SARIF output;
- connect to MCP servers and JavaScript extensions;
- expose text, JSON, RPC, client/server, and Rust embedding surfaces;
- provide optional local CPU speech-to-text in the terminal composer.

### Product surfaces

| Surface | Purpose |
| --- | --- |
| Interactive TUI | Main terminal coding experience |
| --print / -p | Run one prompt non-interactively and exit |
| --mode json | Stream newline-delimited machine-readable events |
| --mode rpc | JSON-RPC over stdio for embedding and integrations |
| davinci server / client | Experimental typed client/server transport |
| Rust library | Programmatic embedding through davinci-coding-agent |
| MCP | Native MCP client |
| JavaScript extensions | Optional Node-hosted compatibility/extension layer |

---

## Installation

DaVinci is currently installed from source. This repository does not currently publish prebuilt GitHub release assets.

### Requirements

For the core CLI:

- Git
- Rust **1.83.0** with Cargo
- rustfmt and clippy for development

The repository pins the toolchain in [rust-toolchain.toml](rust-toolchain.toml). A normal rustup installation will select the required Rust version automatically.

Optional dependencies:

- **Node.js** — required only for JavaScript extensions, selected compatibility features, and some test fixtures. The core Rust CLI does not require Node.
- **CMake and a C++17 compiler** — required to build the native local voice worker.
- **Linux local voice builds** — ALSA development headers and pkg-config.
- **Language intelligence** — may require project-local language-server packages such as TypeScript and typescript-language-server.

### 1. Clone the repository

~~~bash
git clone https://github.com/J12003LPZ/davinci.git
cd davinci
~~~

### 2. Install the core CLI

If you only want the davinci executable:

~~~bash
cargo install --path crates/davinci-coding-agent --locked --force
~~~

Or build without installing:

~~~bash
cargo build --release -p davinci-coding-agent --locked
./target/release/davinci --version
~~~

Windows PowerShell:

~~~powershell
cargo build --release -p davinci-coding-agent --locked
.\target\release\davinci.exe --version
~~~

### 3. Install DaVinci with local voice support

The repository install scripts build both davinci and the matching davinci-voice-worker.

Linux/macOS:

~~~bash
./scripts/install.sh
~~~

Windows PowerShell:

~~~powershell
pwsh .\scripts\install-davinci.ps1
~~~

The scripts install into Cargo's binary directory, normally:

- Linux/macOS: ~/.cargo/bin
- Windows: %USERPROFILE%\.cargo\bin

Make sure that directory is on PATH.

### Linux packages for local voice

On Debian/Ubuntu-family systems:

~~~bash
sudo apt-get update
sudo apt-get install -y build-essential cmake pkg-config libasound2-dev
~~~

### Verify the installation

~~~bash
davinci --version
davinci --help
~~~

---

## Quick start

Start the interactive TUI in the current repository:

~~~bash
davinci
~~~

Start with an initial task:

~~~bash
davinci "Explain this codebase and identify the main entry points."
~~~

Run one task and exit:

~~~bash
davinci -p "Find the cause of the failing tests and explain it."
~~~

Include files or images:

~~~bash
davinci @README.md @screenshot.png "Review these."
~~~

Choose a provider/model:

~~~bash
davinci --model openai/gpt-4o "Review this repository."
~~~

Set a reasoning level:

~~~bash
davinci --thinking high "Investigate this bug."
~~~

Run with a restricted tool surface:

~~~bash
davinci --tools read,grep,find,ls -p "Review the code without changing anything."
~~~

Continue or resume work:

~~~bash
davinci --continue
davinci --resume
~~~

Machine-readable output:

~~~bash
davinci --mode json -p "Run the relevant tests and summarize the result."
~~~

JSON-RPC over stdio:

~~~bash
davinci --mode rpc
~~~

Disable supported startup network work:

~~~bash
davinci --offline
~~~

---

## Models and authentication

DaVinci ships a compiled model/provider catalog and native request/streaming support for multiple provider protocols.

Examples:

~~~bash
davinci --list-models
davinci --list-models claude
davinci --provider openai --model gpt-4o
davinci --model anthropic/claude-sonnet-4
~~~

Supported reasoning-level names are:

~~~text
off
minimal
low
medium
high
xhigh
max
~~~

The effective levels depend on the selected provider and model.

### Credentials

DaVinci can use provider API keys from environment variables and supports OAuth/authentication flows for providers that implement them.

Common variables include:

~~~text
ANTHROPIC_API_KEY
OPENAI_API_KEY
GEMINI_API_KEY
OPENROUTER_API_KEY
XAI_API_KEY
MISTRAL_API_KEY
GROQ_API_KEY
CEREBRAS_API_KEY
AWS_PROFILE
AWS_ACCESS_KEY_ID
AWS_SECRET_ACCESS_KEY
AWS_REGION
~~~

Run davinci --help for the current complete provider/environment list.

When an interactive authentication flow stores credentials, they are written under the resolved DaVinci agent directory.

---

## Permissions and project trust

DaVinci treats tool execution as a controlled runtime boundary.

### Permission modes

| Mode | Behavior |
| --- | --- |
| manual | Ask before protected actions |
| accept-edits | Allow normal edits while retaining stronger gates |
| plan-mode | Read-only planning; mutations are denied |
| auto | Allow lower-risk actions and escalate higher-risk ones |
| always-approve | Remove harness approval prompts while retaining explicit policy and OS boundaries |

Examples:

~~~bash
davinci --permission-mode manual
davinci --permission-mode plan-mode
davinci --permission-mode auto
~~~

The Codex-style --sandbox alias is also accepted:

~~~bash
davinci --sandbox read-only
davinci --sandbox workspace-write
davinci --sandbox full-access
~~~

These are **application policy presets, not an operating-system sandbox**.

### Project trust

Project-local configuration, skills, extensions, and other executable/configurable resources require project trust before DaVinci honors them.

One-run overrides:

~~~bash
davinci --approve
davinci --no-approve
~~~

AGENTS.md and CLAUDE.md can be loaded as repository context. Disable context-file discovery with:

~~~bash
davinci --no-context-files
~~~

---

## Execution modes

DaVinci provides several orchestration models for different task sizes.

### Normal agent

The default interactive turn loop for direct coding tasks, questions, focused fixes, and ordinary repository work.

### One-shot subagents

Bounded isolated workers for delegated research or implementation. Workers use a scoped tool set and isolated context.

### Agent teams

Persistent collaborating agents with typed mailboxes, task tracking, atomic task claims, and event-driven coordination.

### Workflows

Repeatable DAG-based orchestration for structured multi-phase work. Workflows support bounded state, artifact handoff, retries, and fan-out/fan-in join policies.

### Graph engineering

The /graph path is the deterministic engineering pipeline for larger code changes:

~~~text
classify -> investigate -> plan -> implement -> verify -> review
~~~

Graph workers use isolated contexts and explicit role/tool policies. Verification is deterministic: tests and commands succeed or fail by actual exit status rather than by a model deciding that they probably passed.

Graph supports:

- revision loops;
- replanning when a plan becomes invalid;
- worktree isolation;
- execution budgets and deadlines;
- mutation provenance;
- security gates;
- complete review-chunk coverage;
- replay fingerprints and resumable runs;
- bounded memory/skill context injection.

See [Runtime orchestration](docs/runtime-orchestration.md) and [Ecosystem architecture](docs/ecosystem.md).

---

## Major capabilities

### Coding tools

The primary built-in tool surface includes:

- read
- write
- edit
- bash
- powershell
- grep
- find
- ls

Native capabilities and extensions register additional tools.

Tool exposure can be changed per run:

~~~bash
davinci --tools read,grep,find,ls
davinci --exclude-tools bash,powershell
davinci --no-tools
davinci --no-builtin-tools
~~~

### Repository and workspace intelligence

DaVinci can build and reuse structured information about the active checkout instead of repeatedly rediscovering the same facts.

Native engineering subsystems include:

- repository indexing and symbol/module relationships;
- workspace metadata and snapshots;
- package/dependency intelligence;
- build-system intelligence;
- Git intelligence;
- change-impact analysis;
- test-impact analysis;
- verification planning;
- language/framework signals.

Shared engineering snapshots are invalidated on relevant mutations and reused only when workspace identity and observed stamps remain valid.

### Language intelligence

DaVinci can talk to installed language servers through a bounded read-only semantic layer.

The TypeScript integration includes operations such as:

- definition;
- references;
- implementations;
- hover;
- document symbols;
- workspace symbols;
- call hierarchy;
- diagnostics.

DaVinci does not silently install or download a language server. Install the relevant project language-server packages yourself.

See [Language intelligence](docs/language-intelligence.md).

### Test impact and verification planning

The test-impact system identifies relevant tests from observed repository structure and changes. Verification planning combines repository, build, and change facts into a bounded verification strategy.

These systems are planning inputs; actual test/build exit codes remain the authority.

See [Test impact](docs/test-impact.md).

### Managed processes

DaVinci has a native process supervisor for long-lived commands and tool-owned processes, including lifecycle, ownership, cancellation, output capture, and process-tree cleanup.

See [Process manager](docs/process-manager.md).

### Transactional edits

Mutation-heavy workflows can use transaction boundaries that preserve recovery state, execution evidence, and verification information instead of treating file writes as unrelated operations.

See [Transactional edits](docs/transactional-edits.md).

### Token Governor

Large tool results do not need to stay fully resident in model context.

The Token Governor can:

- keep bounded summaries in context;
- preserve exact original output in an artifact store;
- recover exact ranges later through retrieve_output;
- deduplicate unchanged reads;
- detect repeated low-value search/list loops;
- compact structured output through specialized representations.

Compaction changes what remains resident; it does not discard the authoritative stored output.

### Context VM

The optional Context VM compiles a bounded provider working set from authoritative session history while keeping original history as the source of truth.

Rollout modes:

~~~text
off
shadow
active
~~~

It provides:

- explicit provider-context budgets;
- immutable prepared context images;
- fold/checkpoint state;
- protocol-safe live tool exchanges;
- artifact-backed historical evidence;
- source provenance and retrieval;
- admission failure before provider dispatch when required context cannot fit.

The feature is opt-in and can be evaluated in shadow mode before active use.

See [Context VM](docs/context-vm.md).

### Vector memory

DaVinci can retain local repository-scoped memories across sessions and context compaction.

The memory system supports:

- repository-scoped records;
- secret redaction;
- lexical retrieval;
- optional local embeddings;
- optional Ollama/Qdrant acceleration;
- bounded automatic injection;
- direct memory search.

Retrieved memory is treated as untrusted data context, not as new instructions.

### Self-improving learning and skills

Settled turns and verified graph outcomes can be distilled into reusable procedural skills.

The learning system tracks exact skill versions and content hashes. Successful procedures can be reused by later graph runs without allowing learned content to bypass deterministic verification.

See [Learning](docs/learning.md).

### Security scanning

DaVinci includes a native security-analysis pipeline with bounded evidence and auditable output.

Depending on mode and configuration, it can produce:

- candidate and finding records;
- coverage information;
- Markdown reports;
- JSON reports;
- SARIF;
- content hashes and sealed evidence;
- resumable/interrupted checkpoints.

Security analysis does not replace independent review, and an empty report is not a guarantee that a repository is secure.

See [Security scan](docs/security-scan.md).

### Decision intelligence

TypeSafe / Jev is an optional provider-neutral decision-observation layer.

The current Phase 1 implementation is shadow-only: validated observations and telemetry may be collected, but they do not weaken or replace deterministic routing, permissions, verification, security, or workspace boundaries.

It is disabled by default.

See [Decision intelligence](docs/decision-intelligence.md).

### Browser and interaction tooling

The native extension host contains browser/interaction capabilities used by selected verification and engineering workflows. CI includes dedicated cross-platform interaction and browser gates.

### Prompt profiles

Built-in prompt behavior is versioned independently of the model:

~~~bash
davinci --prompt-profile stable
davinci --prompt-profile preview
davinci --prompt-profile legacy-v1
~~~

See [Prompt engineering](docs/prompt-engineering.md) and [Behavioral evaluations](docs/behavioral-evals.md).

---

## Sessions and persistence

DaVinci stores conversations as branchable session history rather than only ephemeral chat state.

Common commands:

~~~bash
davinci --continue
davinci --resume
davinci --session <path-or-id>
davinci --session-id <id>
davinci --fork <path-or-id>
davinci --no-session
~~~

New installations normally use:

~~~text
~/.davinci/agent/sessions/
~~~

If an existing legacy Pi directory is present, DaVinci can continue using:

~~~text
~/.pi/agent/
~~~

Session history is JSONL-compatible. An optional SQLite layer provides derived indexing and branch/fact cache state.

Export a session to standalone HTML:

~~~bash
davinci --export session.jsonl output.html
~~~

---

## Configuration

### User directory resolution

DaVinci resolves its user agent directory in this order:

1. DAVINCI_CODING_AGENT_DIR
2. legacy PI_CODING_AGENT_DIR
3. existing ~/.davinci/agent
4. existing legacy ~/.pi/agent
5. otherwise ~/.davinci/agent

Session-directory overrides use the same DaVinci-first, Pi-compatible policy.

### Common user files

Under the resolved agent directory:

~~~text
settings.json
auth.json
models.json
mcp.json
keybindings.json
sessions/
extensions/
skills/
themes/
workflows/
learning/
voice/
security-scans/
~~~

Not every installation will contain every path.

### Project configuration

Project-local resources are supported under DaVinci/Pi-compatible project directories and are subject to project trust. Depending on the subsystem, project resources may contain settings, graph/workflow configuration, skills, memory, and other scoped state.

### Useful environment variables

~~~text
DAVINCI_CODING_AGENT_DIR
DAVINCI_CODING_AGENT_SESSION_DIR
PI_CODING_AGENT_DIR
PI_CODING_AGENT_SESSION_DIR
PI_OFFLINE
PI_NODE
~~~

Provider-specific credentials use the provider's normal environment variables.

---

## Extensions and MCP

### JavaScript extensions

JavaScript extensions run in a Node subprocess and can add:

- tools;
- slash commands;
- CLI flags;
- autocomplete providers;
- model/provider integrations;
- authentication flows;
- custom rendering/input behavior.

Node is optional unless this layer is used.

Package-management commands include:

~~~bash
davinci install <source>
davinci remove <source>
davinci list
davinci config
~~~

### MCP

DaVinci includes a native Model Context Protocol client.

User MCP configuration is loaded from the resolved agent directory, normally:

~~~text
~/.davinci/agent/mcp.json
~~~

Project-local MCP/configuration is subject to trust and permission policy.

See [davinci-mcp](crates/davinci-mcp).

---

## Local voice input

Local voice input is experimental and is available in the default interactive DaVinci terminal composer.

It uses a separate davinci-voice-worker and local CPU speech recognition. Dictation inserts editable text at the cursor; it does **not** automatically send or execute the transcription.

Useful commands:

~~~bash
davinci voice status
davinci voice devices
davinci voice model list
davinci voice model install base
davinci voice model import base /path/to/ggml-base.bin
~~~

Supported local model sizes are tiny, base, and small.

For setup, privacy behavior, platform requirements, model verification, and current limitations, see [Local voice input](docs/voice-input.md).

---

## Architecture

DaVinci is a 14-crate active Rust workspace.

~~~mermaid
flowchart TD
    CLI["CLI / TUI / SDK / RPC"] --> Agent["Agent runtime"]
    CLI --> UI["davinci-tui"]

    Agent --> AI["Provider + model runtime"]
    Agent --> Policy["Permissions / contracts / trust"]
    Agent --> Runtime["Tasks / teams / workflows / graph"]
    Agent --> Context["Context VM / compaction / governor"]

    Policy --> Tools["Built-ins / native capabilities / MCP / JS extensions"]
    Runtime --> Tools
    Tools --> Agent

    Agent --> Sessions["JSONL sessions / SQLite derived state"]
    Agent --> Evidence["Artifacts / receipts / verification evidence"]
    Agent --> UI
~~~

Primary crate responsibilities:

| Crate | Responsibility |
| --- | --- |
| davinci-coding-agent | CLI, startup, TUI integration, settings/trust, extensions, SDK |
| davinci-agent | Turns, tools, permissions, planning, jobs, orchestration, context/evidence runtime |
| davinci-ai | Models, auth/OAuth, provider requests, retries, streaming, usage |
| davinci-tui | Terminal UI, editor, instruments, themes, voice UI state |
| davinci-voice | Local speech worker and audio contracts |
| davinci-session | Session discovery, JSONL history, branches |
| davinci-session-sqlite | SQLite persistence, migrations, branch/fact caches |
| davinci-mcp | MCP client and transports |
| davinci-protocol | Typed IPC schemas and CBOR framing |
| davinci-client | Client transport and handshake |
| davinci-server | Background session/controller server |
| davinci-telemetry | Runtime telemetry contracts |
| davinci-evals | Behavioral and engineering evaluation harnesses |

For entry points and control flow, start with [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

---

## Repository layout

~~~text
davinci/
├── crates/
│   ├── davinci-coding-agent/   # Main CLI and product assembly
│   ├── davinci-agent/          # Agent/runtime/orchestration engine
│   ├── davinci-ai/             # Providers, models, auth, streaming
│   ├── davinci-tui/            # Terminal interface
│   ├── davinci-voice/          # Local voice worker
│   └── ...                     # Protocol, sessions, MCP, server, evals, etc.
├── docs/                       # Architecture and capability documentation
├── scripts/                    # Installation and validation scripts
├── vendor/davinci/             # Pinned TypeScript behavioral reference
├── .github/workflows/          # CI, behavior, security, lint, evaluation workflows
├── Cargo.toml                  # Rust workspace
├── Cargo.lock                  # Locked dependency graph
├── rust-toolchain.toml         # Rust 1.83.0
└── README.md
~~~


---

## Development

### Build

~~~bash
cargo build -p davinci-coding-agent
~~~

Release build:

~~~bash
cargo build --release -p davinci-coding-agent --locked
~~~

### Test

All active workspace crates:

~~~bash
cargo test --workspace
~~~

Static checks:

~~~bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
~~~

Repository-pinned/offline validation, once dependencies are cached:

~~~bash
cargo check --workspace --all-targets --offline --locked
cargo clippy --workspace --all-targets --offline --locked -- -D warnings
~~~

Exported-session viewer test:

~~~bash
node --test crates/davinci-coding-agent/export-html/template.test.cjs
~~~

### Make targets

~~~bash
make build
make test
make fmt
make clippy
make install
~~~

### Evaluation suites

The repository includes deterministic evaluation and regression infrastructure for prompt behavior, orchestration, engineering tools, security, browser behavior, parity, and runtime invariants.

Examples:

~~~bash
cargo test -p davinci-evals behavior::
cargo test -p davinci-coding-agent ecosystem_loop_ -- --nocapture
cargo test -p davinci-coding-agent ecosystem_invariants_ -- --nocapture
~~~

Some tests intentionally require external software, a browser, language server, audio stack, or live provider credentials. Offline fixture tests do not prove those external integrations.

---

## Documentation

Start here:

- [Architecture and navigation](docs/ARCHITECTURE.md)
- [Documentation index](docs/README.md)
- [Runtime orchestration](docs/runtime-orchestration.md)
- [Context VM](docs/context-vm.md)
- [Closed ecosystem integration](docs/ecosystem.md)
- [Repository intelligence](docs/repo-intelligence.md)
- [Language intelligence](docs/language-intelligence.md)
- [Test impact](docs/test-impact.md)
- [Managed processes](docs/process-manager.md)
- [Transactional edits](docs/transactional-edits.md)
- [Security scan](docs/security-scan.md)
- [Learning](docs/learning.md)
- [Local voice input](docs/voice-input.md)
- [Decision intelligence](docs/decision-intelligence.md)
- [Prompt engineering](docs/prompt-engineering.md)
- [Behavioral evaluations](docs/behavioral-evals.md)

Historical implementation plans and archived milestones live under docs/superpowers and docs/archive. Current code and current capability guides are the source of truth for shipped behavior.

---

## Compatibility

DaVinci intentionally retains compatibility with parts of the original Pi ecosystem.

Compatibility includes:

- legacy PI_* environment variables;
- discovery of existing ~/.pi/agent state;
- Pi-style session/history layout;
- project resource compatibility;
- JavaScript extension hosting;
- preserved TypeScript behavioral reference fixtures.

New DaVinci installations prefer:

~~~text
~/.davinci/agent
DAVINCI_CODING_AGENT_DIR
DAVINCI_CODING_AGENT_SESSION_DIR
~~~

Existing legacy state does not need to be migrated immediately.

---

## Security notes

DaVinci can execute model-requested shell commands and can modify files when the active permission policy allows it.

Important boundaries:

- approval modes are application-level policy, not an OS sandbox;
- project-local executable/configurable resources are protected by project trust;
- credentials should never be committed to the repository;
- language servers, MCP servers, extensions, build tools, browsers, and child processes run with the user's OS permissions unless separately sandboxed;
- security-scan results are evidence, not a guarantee that vulnerabilities are absent;
- use --offline when you explicitly want supported startup network operations disabled.

Review [docs/security](docs/security/) before deploying DaVinci in a sensitive environment.

---

## License

DaVinci is licensed under the [MIT License](LICENSE).

The repository also contains vendored/reference components with their own licenses and notices. Local voice native dependencies and model provenance are documented under [crates/davinci-voice](crates/davinci-voice) and [docs/voice-input.md](docs/voice-input.md).
