# DaVinci

[![CI](https://github.com/J12003LPZ/davinci/actions/workflows/ci.yml/badge.svg)](https://github.com/J12003LPZ/davinci/actions/workflows/ci.yml)
[![Workflow lint](https://github.com/J12003LPZ/davinci/actions/workflows/workflow-lint.yml/badge.svg)](https://github.com/J12003LPZ/davinci/actions/workflows/workflow-lint.yml)
[![Security SARIF](https://github.com/J12003LPZ/davinci/actions/workflows/security-sarif.yml/badge.svg)](https://github.com/J12003LPZ/davinci/actions/workflows/security-sarif.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

**DaVinci is a native Rust AI coding-agent harness for the terminal.**

> **Current terminal:** see the [implementation and review guide](docs/ui/terminal-rebuild.md) for the default shell, settings/model selectors, optional graph view, rendered previews, and verification limitations.

It combines an interactive coding assistant, multi-provider model runtime, permission system, persistent sessions, engineering intelligence, multi-agent orchestration, deterministic verification, security analysis, memory and learning, MCP, extensions, and optional local voice input in one CLI.

DaVinci began as a Rust-compatible rewrite of the TypeScript [pi](https://github.com/earendil-works/pi) coding agent and has grown into a larger native harness. The pinned TypeScript source under [vendor/davinci](vendor/davinci) remains a behavioral compatibility reference. The active product is the Rust workspace in this repository.

> **Workspace version:** 1.0.71
> **Rust toolchain:** 1.83.0  
> **Primary executable:** `davinci`  
> **Documentation baseline:** `main` at [`0a57e476`](https://github.com/J12003LPZ/davinci/commit/0a57e476c2088250299438c91d582571614f0687), including merged [PR #88](https://github.com/J12003LPZ/davinci/pull/88)

The version string alone does not identify the installed source: different commits can report **1.0.71**. Features described here require a binary built from the corresponding source. Rebuild and restart after updating; use `/doctor` to inspect the actual installation identity.

---

## Contents

- [What DaVinci does](#what-davinci-does)
- [Installation](#installation)
- [Quick start](#quick-start)
- [Models and authentication](#models-and-authentication)
- [Command reference](#command-reference)
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
- [Troubleshooting and limitations](#troubleshooting-and-limitations)
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
| davinci server / client | Experimental Unix transport; requires an `experimental-ipc` build |
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
- **Language intelligence** — optional local servers: TypeScript/typescript-language-server, rust-analyzer, and BasedPyright or Pyright. DaVinci discovers installed tools but never installs them automatically; see [docs/language-intelligence.md](docs/language-intelligence.md).

### 1. Clone the repository

~~~bash
git clone https://github.com/J12003LPZ/davinci.git
cd davinci
~~~

### 2. Build or install the core CLI from source

For a development/source installation of only the `davinci` executable:

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

A direct Cargo build/install does not produce the CI-backed installation identity used by the release scripts. Keep the checkout commit with your test results; do not describe a source build as a verified release.

### 3. Verified installation with local voice support

The repository install scripts build both `davinci` and the matching `davinci-voice-worker`, then install both with identity sidecars. They are **release-gated**, not a shortcut for installing any arbitrary `main` checkout.

In addition to Rust and voice build dependencies, they require Python 3 and an authenticated GitHub CLI (`gh`). Before building they verify:

- a clean, committed checkout;
- the exact product-version tag (`v<workspace-version>`) points at that commit;
- completed green full CI and workflow-lint evidence for that exact commit.

Fetch tags and select the intended tagged checkout before running them. A missing tag, dirty checkout, missing GitHub access, or non-green/missing CI makes preflight fail. See [release identity enforcement](scripts/release_identity.py) and [release discipline](docs/readiness/release-discipline.md).

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

Inside the TUI, run `/doctor` and `/status`. Check which binary your shell resolves with `command -v davinci` on Linux/macOS or `Get-Command davinci` in PowerShell.

### Update an existing installation

For a source checkout, preserve local changes, update with `git pull --ff-only`, then repeat the appropriate build/install path above and restart. A release-script installation must still satisfy the tag and CI gates.

`davinci update` updates extension packages by default. `davinci update --models` refreshes model catalogs. **`davinci update self` is currently unsupported** and does not rebuild this Rust CLI; see [package dispatch](crates/davinci-coding-agent/src/packages.rs).

---

## Quick start

For ChatGPT subscription use, complete [the OpenAI Codex login](#chatgpt-subscription-openai-codex) first. Use a model that your account actually offers.

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
davinci --model openai-codex/gpt-6-astra "Review this repository."
~~~

Set a reasoning level:

~~~bash
davinci --thinking high "Investigate this bug."
~~~

Run a read-only review with a restricted tool surface:

~~~bash
davinci --permission-mode plan-mode --tools read,grep,find,ls -p "Review the code without changing anything."
~~~

A tool allowlist controls exposure; the permission mode supplies the read-only policy.

Continue or resume work:

~~~bash
davinci --continue
davinci --resume
~~~

Machine-readable output:

~~~bash
davinci --mode json -p "Run the relevant tests and summarize the result."
~~~

Scripted runs in another directory, keeping the final reply in a file:

~~~bash
davinci -C ../service -o last-reply.txt -p "Fix the failing test."
~~~

These two flags are Davinci additions for Codex `exec` parity. TypeScript pi has neither.

- `--cd, -C <dir>` runs as if davinci had been started in `<dir>`. It applies before settings, project trust, AGENTS.md discovery and session-directory resolution, so the session is stored under that directory's encoding. Relative `@file` and `--session` paths resolve against it. A missing path or a file is an error.
- `--output-last-message, -o <file>` writes the final assistant reply text to `<file>` after a `--print` or `--mode json` run. The write is atomic (temporary file, then rename). It happens even when the run fails or is blocked, with an empty file when there is no reply. A relative path resolves against the directory davinci was started in, not the `--cd` directory. If the write fails, davinci prints an error and exits 1, unless the run already failed with its own code. Interactive and `--mode rpc` runs reject the flag.

A final answer that must be JSON of a known shape:

~~~bash
davinci --output-schema report.schema.json -o report.json -p "Summarize the open issues."
~~~

`--output-schema <file>` is also a Codex `exec` parity addition. It works with `--print` and `--mode json`.

- The file must hold a JSON object, the JSON schema. A missing file or invalid JSON is an error before any model call. A relative path resolves against the launch directory, like `-o`.
- OpenAI Responses routes (`openai-responses`, `openai-codex-responses`, `azure-openai-responses`) send the schema as `text.format` with `strict: true`. Chat completions routes send it as `response_format`. The schema is passed through as written, so a provider that rejects it fails the run with its own error. Every provider receives the same schema in its final-answer instructions, including repair requests; non-OpenAI providers do not receive an OpenAI-specific wire parameter.
- Every provider's final answer is then checked. The answer may be surrounded by whitespace or be one fenced `json` block. Davinci checks `type`, `properties`, `required`, `additionalProperties`, `items`, `minItems`, `maxItems`, `enum`, `const` and `anyOf`. Unsupported assertion keywords (including `$ref`, `pattern`, `format`, and numeric bounds) and malformed supported keywords are rejected before a request; they are never silently ignored. Descriptive annotations are permitted.
- Schemas are capped at 32 KiB and 32 nested schema levels. Replies are capped at 256 KiB. Validation stops after 10,000 visits and fails closed on exhaustion. Repair diagnostics contain at most 20 errors of 1,024 characters each. Numeric literals that change value when parsed are rejected rather than validated against a rounded value.
- A mismatch gets exactly one repair turn: the model is told what failed and asked for only the corrected JSON. If that answer still fails, davinci lists the errors on stderr and exits 1.
- On success stdout and the `-o` file hold the bare JSON, without a fence. `--mode json` ends with an `output_schema` event (`checked`, `valid`, `repairTurns`, `errors`). The repair turn is also counted as `outputSchemaRepairTurns` in the run stats.

JSON-RPC over stdio:

~~~bash
davinci --mode rpc
~~~

Offline harness smoke test:

~~~bash
davinci --offline --no-mcp --no-extensions -p "Check the terminal path."
~~~

**Offline is a fixture path, not local-model inference.** In the ordinary agent loop, `--offline` (or `PI_OFFLINE=1`) replaces the live provider response with a deterministic stub such as `(offline) received 24 characters`. It also disables supported startup network work. It cannot validate login, model reasoning, live provider behavior, or real task completion, and it is not an OS network sandbox. See [provider dispatch and offline stub](crates/davinci-coding-agent/src/main.rs).

---

## Models and authentication

### ChatGPT subscription (`openai-codex`)

This is the repository's primary workflow. **A ChatGPT subscription login and an OpenAI API key are different authentication routes.**

1. Start `davinci`.
2. Enter `/login openai-codex`.
3. Follow the printed authorization URL and choose **Continue with ChatGPT**. The interactive flow waits up to five minutes for a callback on `http://127.0.0.1:1455/auth/callback`, on the machine running DaVinci.
4. Wait for the credential-stored confirmation. Merely opening the URL is not a successful login.
5. Use `/model` to choose an available `openai-codex` model, or specify one explicitly on the next launch.

~~~bash
davinci --list-models openai-codex
davinci --provider openai-codex --model gpt-6-astra --thinking high
davinci auth check --provider openai-codex --json --no-refresh
~~~

The model above is in the source catalog; catalog presence does not guarantee account access. `/model` and `davinci --list-models` show the catalog visible to your installation. Refresh with `davinci update --models` when needed.

Current `main` uses DaVinci's Sign in with ChatGPT flow, validates the issued registration and identity, and requires the `chatgpt.tokens.use.direct` scope. Its `openai-codex` catalog targets `https://api.openai.com/v1` using the subscription OAuth credential. The endpoint name does not make this an API-key workflow. The implementation is in [openai_siwc.rs](crates/davinci-ai/src/openai_siwc.rs), [OAuth providers](crates/davinci-ai/src/oauth_providers.rs), and [login dispatch](crates/davinci-coding-agent/src/main.rs).

Important boundaries:

- No `OPENAI_API_KEY` is required for `openai-codex`; API keys are explicitly rejected for this provider.
- Legacy Codex tokens or copied credentials are not a substitute for the verified DaVinci login. Older saved credentials may require signing in again.
- Do not change to `--provider openai` as a login workaround unless you intentionally want the separate API-key route.
- Browser/account/backend support is required. Source code and offline tests do not establish a successful live authorization or a working subscription entitlement.
- A browser on another machine cannot reach the running CLI's loopback callback directly. If the flow reports a callback or registration error, preserve the non-secret error and retry the supported login on the same machine. Do not paste credentials or authorization URLs into issues.
- `auth check` checks credential readiness, not a live model completion. It refreshes expired OAuth credentials unless `--no-refresh` is supplied.

### Other providers and API-key use

The compiled catalog includes OpenAI API, Anthropic, Google, Azure OpenAI, Amazon Bedrock, GitHub Copilot, OpenRouter, and other providers. Protocol support, available models, credentials, and service terms differ by provider; use the [catalog](crates/davinci-ai/src/catalog.rs) and `davinci --help` for the current list.

For intentional API-key use:

~~~bash
davinci --provider openai --model gpt-4o
davinci --model anthropic/claude-sonnet-4
~~~

Supply the selected provider's credential through its environment variable or supported login flow. Common variables include `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GEMINI_API_KEY`, `OPENROUTER_API_KEY`, `XAI_API_KEY`, `MISTRAL_API_KEY`, `GROQ_API_KEY`, and `CEREBRAS_API_KEY`. Bedrock supports its configured AWS credentials/profile. Avoid putting secrets in command-line arguments or committed settings.

A local llama.cpp server can be configured with `/login llama.cpp http://127.0.0.1:8080` or `LLAMA_BASE_URL`. This still requires an actual running model server; `--offline` does not start one or run inference.

Credentials saved by login live in `auth.json` under the [resolved agent directory](#user-directory-resolution). `/logout <provider>` removes stored authentication; it does not unset shell environment variables or remove credentials configured in `models.json`.

### Models, reasoning, speed, and usage

~~~bash
davinci --list-models
davinci --list-models claude
davinci --model openai-codex/gpt-6-astra:high
~~~

Reasoning levels are `off`, `minimal`, `low`, `medium`, `high`, `xhigh`, and `max`; support and mapping depend on the model. Use `/thinking <level>` or its alias `/effort <level>` interactively.

`/fast` toggles Fast/Standard for OpenAI Codex while keeping the selected model and reasoning level. It requests a service tier, not a guaranteed latency result. Availability depends on the model/backend/account; Fast may consume subscription limits faster. See [configuration](#useful-environment-variables).

Use `/status`, `/cost`, and `/context` to inspect session state and usage. USD estimates and cached-token counts do not measure remaining included subscription allowance. Missing usage is unknown, not zero. See [subscription efficiency](docs/openai-efficiency.md).

### Credential command safety

~~~bash
davinci auth help
davinci auth check --provider openai-codex --json
~~~

`auth print-api-key`, `auth print-bearer-token`, and `auth check --credentials` deliberately print secrets. Use them only for a deliberate private integration; never include their output in screenshots, logs, bug reports, or chat. They are unnecessary for ordinary DaVinci use.

---

## Command reference

Shell commands below run in your terminal. Slash commands run inside the DaVinci composer. Extension, skill, and prompt-template commands depend on what is installed and trusted.

For your actual binary, start with `davinci --help`, `davinci plugin help`, `davinci auth help`, and interactive `/help` / `/hotkeys`. The reference below is checked against [argument parsing](crates/davinci-coding-agent/src/args.rs), [CLI help](crates/davinci-coding-agent/src/help.txt), [built-in slash commands](crates/davinci-coding-agent/src/slash.rs), and [native commands](crates/davinci-coding-agent/src/native_extensions/mod.rs). Maintenance and feature-gated commands have their own parsers.

### CLI options

| Area | Options | What to know |
| --- | --- | --- |
| Model | `--provider <name>`, `--model <id-or-pattern>`, `--thinking <level>` | Model accepts `provider/id` and an optional `:thinking` suffix. |
| Model cycling | `--models <patterns>` | Comma-separated model patterns. |
| API credentials | `--api-key <key>` | For supported API routes, not `openai-codex`; prefer a private environment variable. |
| Prompts | `--system-prompt <text>`, `--append-system-prompt <text-or-file>`, `--prompt-profile stable` | Append is repeatable; profiles also include `preview` and `legacy-v1`. |
| Input | `@file`, `@image`, `--` | Attach files to the initial message; `--` ends option parsing. RPC rejects CLI file attachments. |
| Output | `--print` / `-p`, `--mode text`, `--mode json`, `--mode rpc` | JSON streams events; RPC uses stdio. |
| Final output | `--output-last-message <file>` / `-o`, `--output-schema <file>` | Non-interactive runs only; see [quick start](#quick-start) for schema limits and path rules. |
| Directory | `--cd <dir>` / `-C`, `--add-dir <dir>` | Extra writable directories must already exist; `--add-dir` is repeatable and does not override denials or worker isolation. |
| Approval | `--permission-mode <mode>`, `--sandbox <preset>` | Approval presets; separate from OS isolation. |
| OS isolation | `--execution-sandbox <mode>` | `none`, `restricted`, `workspace-write`, `full-access`; see [permissions](#permissions-and-project-trust). |
| Non-interactive approval | `--approval-policy abort`, `--approval-policy deny-continue`, `--fail-on-denied` | Does not grant permission; details below. |
| Budget | `--root-budget <file>` | Durable shared request/time admission budget; [configuration contract](docs/readiness/openai-harness-implementation.md#budget-operation). Subscription budgets do not claim an API-dollar limit. |
| Sessions | `--continue` / `-c`, `--resume` / `-r`, `--session <path-or-id>`, `--session-id <id>`, `--fork <path-or-id>` | Continue latest, choose, address, or fork a session. |
| Session storage | `--session-dir <dir>`, `--no-session`, `--name <name>` / `-n` | Override location, avoid conversation persistence, or name a session. |
| Tool selection | `--tools <names>` / `-t`, `--exclude-tools <names>` / `-xt` | Comma-separated allow/deny lists. |
| Disable tools | `--no-tools` / `-nt`, `--no-builtin-tools` / `-nbt` | All tools off by default, or built-ins off while retaining custom tools. |
| Extensions/MCP | `--extension <path>` / `-e`, `--no-extensions` / `-ne`, `--no-mcp` | Disabling JS discovery does not disable native Rust intelligence. |
| Skills/prompts | `--skill <path>`, `--prompt-template <path>`, `--no-skills` / `-ns`, `--no-prompt-templates` / `-np` | Explicit file/directory options are repeatable. |
| Themes | `--theme <path>`, `--use-theme <name[/name]>`, `--no-themes` | Load a theme resource versus select an initial theme. |
| Project trust | `--approve` / `-a`, `--no-approve` / `-na`, `--no-context-files` / `-nc` | Trust/ignore project resources for this run; context discovery is separately switchable. |
| Terminal | `--tui-mode regular`, `--tui-mode fullscreen`, `--legacy-tui`, `--verbose` | Terminal mode, previous chrome, or verbose startup. |
| Utilities | `--export <session-file>`, `--list-models [query]`, `--offline`, `--help` / `-h`, `--version` / `-v` | Offline runs use the deterministic fixture response, not real inference. |

### Non-interactive approvals and exit status

Print/JSON runs cannot answer an interactive approval dialog. The default `--approval-policy abort` reports `approval_required` and exits **1** when an action needs approval. `deny-continue` returns the denial to the model and continues; repeated denials of the same action are bounded. Add `--fail-on-denied` to make an otherwise successful run with denied actions exit **3**.

~~~bash
davinci --permission-mode plan-mode --approval-policy deny-continue --fail-on-denied \
  --mode json -p "Inspect this repository and report what you can verify."
~~~

A final assistant reply is not proof that every requested action ran. Check the exit status, denial events, and actual verification evidence. Model/provider/output-validation failures can also exit **1**. Runtime inspection has its own exit-code meaning described below.

### Shell subcommands

| Command | Purpose / caveat |
| --- | --- |
| `davinci install <source> [-l]` | Install an extension package and record it in settings; `-l` selects project-local scope. |
| `davinci remove <source> [-l]` | Remove an extension package; `uninstall` is an alias. |
| `davinci list`, `davinci config [-l]` | List configured extensions or choose enabled package resources. |
| `davinci update [source]` | Update extension packages, optionally one source. |
| `davinci update --models [provider]` | Refresh model catalogs. |
| `davinci update --extensions`, `davinci update --all` | Update extensions; `--all` updates models and extensions, not the Rust executable. |
| `davinci update self` | Currently returns an unsupported-self-update error; rebuild from source instead. |
| `davinci plugin help` | Plugin/marketplace operations listed below. |
| `davinci auth check --provider <provider> [--json] [--no-refresh]` | Check credential readiness without printing secrets. |
| `davinci voice status`, `davinci voice devices`, `davinci voice model list` | Inspect local voice prerequisites/devices/models. |
| `davinci voice model install <tiny\|base\|small>` | Download an approved voice model. |
| `davinci voice model import <id> <path>` | Verify and import an existing local model. |
| `davinci inspect run <uuid> [--json]` | Read runtime evidence for a run. |
| `davinci inspect operation <uuid> [--json]` | Read one operation's evidence. |
| `davinci inspect session <id> [--json]` | Read session runtime evidence. |
| `davinci doctor runtime [--json]` | Read-only runtime health/evidence inspection; distinct from TUI `/doctor`. |
| `davinci design ...`, `davinci design-sync ...` | Experimental design artifacts; put global flags before the command and read [setup/limits](docs/design-artifacts.md). |
| `davinci server --listen unix:///path`, `davinci client --connect unix:///path` | Experimental Unix-only IPC; compile with `--features experimental-ipc`. Not enabled in the default build. |

The [runtime inspector](crates/davinci-coding-agent/src/runtime_inspect.rs) is read-only. Exit **3** means unavailable or inconsistent evidence, including invalid inspection input; it is not permission to delete journals or retry a possibly completed mutation. Follow the [recovery playbook](docs/runtime/recovery-playbook.md).

### Interactive slash commands

| Command | Purpose |
| --- | --- |
| `/help`, `/hotkeys` | Discover commands and the active keyboard bindings. |
| `/init [focus]` | Ask the agent to inspect the project and create/update `AGENTS.md`; normal model, tool, and permission rules apply. |
| `/setup check` | Check workspace setup without applying the setup changes. |
| `/setup`, `/setup trust` | Apply workspace setup or explicitly trust project resources. Setup can update `.gitignore`, memory/index state, and local service/model setup; review the output first. |
| `/settings`, `/config` | Open the settings panel. |
| `/model [provider/model]` | Open model selection or switch directly. |
| `/thinking <level>`, `/effort <level>` | Select reasoning effort. |
| `/fast` | Toggle requested Fast/Standard speed for OpenAI Codex. |
| `/login [provider]`, `/logout [provider]` | Choose/configure or remove stored provider authentication. |
| `/permissions [mode]` | Inspect or change the approval policy. |
| `/plan`, `/plan show`, `/plan diff` | Enter read-only planning or inspect its revisions. |
| `/plan edit <id> <text>`, `/plan accept <id>`, `/plan reject <id>` | Edit/accept/reject plan decisions; `all` selects all decisions. |
| `/plan approve`, `/plan accept [mode]`, `/act` | Approve a complete plan, approve and choose execution mode, or leave planning without implicit approval. |
| `/new`, `/resume`, `/name <name>` | Start, switch, or name sessions. |
| `/tree`, `/fork`, `/clone` | Navigate branches, fork from an earlier prompt, or duplicate the current position. |
| `/rewind` | Restore code, conversation, or both from a recent prompt; inspect the proposed restore before accepting. |
| `/compact [instructions]` | Manually compact context. |
| `/context [inspect]` | Inspect usage or the prepared-context manifest. |
| `/export [path]`, `/import <path>` | Export HTML/JSONL or import/resume JSONL. |
| `/share` | Upload a session as a secret GitHub gist. Review/redact first: a secret gist is accessible to anyone with its URL. |
| `/copy` | Copy the latest assistant reply. |
| `/reload` | Reload keybindings, extensions, skills, prompts, themes, and context files. |
| `/mcp`, `/status`, `/doctor`, `/cost` | Inspect connected services, session state, setup/install identity, and usage. |
| `/sandbox-status` | Inspect execution-isolation policy and evidence. |
| `/agents [msg <name> <text>\|stop <name>]`, `/tasks` | Inspect profiles/live teams, message/stop a worker, or inspect task state. |
| `/workflow [status <id>\|cancel <id>]` | Inspect workflow runs; available when dynamic workflows are enabled. |
| `/graph [goal]` | Start a graph for a goal or inspect the current run. Also supports `save <name>`, `run <name>`, `--simple`, `--complex`, and `--dry-run`; consult the live graph controls for run-specific actions. |
| `/security-scan [path] [--changed\|--diff base..head] [--mode quick\|standard\|deep]` | Start/inspect/resume security analysis; `--new`, `--report`, `--finding <id>`, and `--format terminal\|json\|sarif` are also supported. |
| `/memory-search <query>`, `/memory-page [--no-open] [query]` | Search durable local memory or inspect it in a generated page. |
| `/memory-reindex`, `/memory-clear` | Rebuild memory indexing/embeddings or clear local records; the latter is destructive. |
| `/governor-reset` | Reset the session's Token Governor state. |
| `/learning-pending`, `/learning-approve <id\|all>`, `/learning-reject <id\|all> [reason]` | Review and accept/reject learned skill candidates. |
| `/skill-list [query]`, `/skill-view <name> [file]` | Find or read reusable skills. |
| `/plugin ...` | The plugin operations below, inside the TUI. |
| `/design ...`, `/design-sync ...` | Feature-gated [design artifact operations](docs/design-artifacts.md#commands-and-controls). |
| `/quit` | Exit; `/exit` and `/q` are aliases. |

`/sessions` and `/session` are compatibility aliases for `/resume`; `/session info` and `/session stats` inspect the session. `/workflows` and `/plugins` are also accepted aliases. Native service diagnostics are consolidated in `/status`; older commands such as `/cache-status`, `/lsp-status`, `/repo-index-status`, `/memory-status`, and `/governor-status` remain internal/compatibility entry points rather than the public command-menu inventory.

Keyboard bindings can be customized. In the default model picker, Enter persists the default and `s` selects for the current session only. Shift+Tab cycles permission modes at the idle composer. Use `/hotkeys` rather than assuming shortcuts are identical across the default and legacy TUIs.

### Plugin commands

These work as `davinci plugin ...` in the shell or `/plugin ...` in the TUI:

~~~text
list
browse [query]
install <name>[@marketplace]
import [claude|codex] [<key>|--all]
info <plugin>
approve <plugin>
revoke <plugin>
test <plugin>
enable <plugin>
disable <plugin>
update <plugin>
uninstall <plugin>
marketplace add <owner/repo|git-url|directory>
marketplace list
marketplace update [name]
marketplace remove <name>
~~~

A bare `plugin import` lists candidates; an explicit key or `--all` adopts them. Installing/enabling a plugin is separate from approving its executable hooks. `approve` allows the listed hooks, `revoke` withdraws that approval, and `test` executes approved SessionStart hooks once. Changes apply after `/reload` or in a new session. Read [plugin behavior and limitations](docs/plugins.md) before importing third-party code.

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

These are **application policy presets, not an operating-system sandbox**. OS execution policy is configured separately:

~~~bash
davinci --permission-mode manual --execution-sandbox restricted
davinci --permission-mode manual --execution-sandbox workspace-write
~~~

`restricted` requests read-only execution; `workspace-write` requests workspace writes with protected control paths and network denied by default. `none` means **no subprocess execution**, not “turn off isolation”; `full-access` is an explicit host escape hatch and makes no filesystem/network isolation claim.

Without an explicit execution policy, Auto attempts a bounded native capability probe and activates workspace-write isolation with network denied when supported. Current Auto-default activation requires usable Linux bubblewrap; native Windows should use an appropriately configured WSL2 environment for this path. Unsupported systems can retain approval policy without active isolation. Earlier unowned JS/MCP execution can prevent later activation, so start directly in Auto when that boundary is required.

Always inspect `/sandbox-status` and `/status` instead of assuming confinement. Approval does not widen an active sandbox. See [sandbox modes, platform coverage, and limitations](docs/sandbox.md).

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

Persistent collaborating agents with typed mailboxes, task tracking, atomic task claims, and event-driven coordination. Opt in with `"agentTeams": true` in user settings or the corresponding `/settings` control; the default is off.

### Workflows

Repeatable DAG-based orchestration for structured multi-phase work. Workflows support bounded state, artifact handoff, retries, and fan-out/fan-in join policies. Opt in with `"dynamicWorkflows": true`; the default is off. See [subagents, teams, and workflow controls](docs/agent-teams.md), including how to prohibit delegation.

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

DaVinci can talk to installed language servers through a bounded read-only semantic layer shared across TypeScript/JavaScript, Rust, and Python.

The stable native surface provides eight operations:

- definition;
- references;
- hover;
- document symbols;
- workspace symbols;
- implementations;
- type definition;
- document diagnostics.

Diagnostics are advisory semantic evidence, not compiler/test verification. Rename, automatic code actions, and call hierarchy are outside the completed read-only scope.

DaVinci does not silently install or download a language server, Rust component, Python environment, or interpreter. Provision compatible local tools separately.

See [Language intelligence](docs/language-intelligence.md) and the [compatibility report](docs/language-intelligence-compatibility.md).

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

Its optional observations/advice do not replace deterministic permissions, verification, security, or workspace boundaries. Individual advice features have separate configuration and readiness constraints; consult the current guide before enabling them.

It is disabled by default.

See [Decision intelligence](docs/decision-intelligence.md).

### Design artifacts (experimental)

`/design` manages session-owned UI concepts, revisions, evidence, exports, and reviewed implementation proposals. It is disabled by default. Generation requires an explicitly enabled feature, a selected `openai-codex` subscription/model/effort, and an existing root-budget configuration. The browser companion additionally needs a separately prepared pinned Node/Chromium runtime; native generated-content rendering currently fails closed on Windows. No automatic API-key fallback or package installation occurs.

Start with [design setup, commands, and limitations](docs/design-artifacts.md) and [readiness](docs/readiness/design-artifacts.md), not an ordinary `/design new` invocation without prerequisites.

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

Session history is JSONL-compatible. An optional SQLite layer provides derived indexing and branch/fact cache state. `--no-session` disables conversation persistence, not every cache, credential, diagnostic, or external-service side effect. Review exports before sharing; they may contain repository content and tool output.

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

Session lookup/storage precedence is `--session-dir`, then `DAVINCI_CODING_AGENT_SESSION_DIR` (legacy `PI_CODING_AGENT_SESSION_DIR`), then `settings.json`'s `sessionDir`, then `<agent-dir>/sessions`. Sessions are grouped by encoded working directory, so `--cd` affects which sessions you find. See [path resolution](crates/davinci-session/src/discovery.rs).

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

User settings live at `<agent-dir>/settings.json`. Project files resolve from `.davinci/<name>` first, then legacy `.pi/<name>`, per file. Directory resources such as skills, prompts, agents, and extensions can be merged from both locations. Trusted project settings overlay user settings subject to field-specific restrictions; project data cannot grant itself trust or widen the execution sandbox.

A minimal user-level subscription configuration:

~~~json
{
  "defaultProvider": "openai-codex",
  "defaultModel": "gpt-6-astra",
  "defaultThinkingLevel": "high",
  "serviceTier": "standard",
  "effortPolicy": "fixed",
  "toolSurface": "full",
  "autoVerify": true,
  "agentTeams": false,
  "dynamicWorkflows": false
}
~~~

Select a model available to your account; these are example choices, not mandatory defaults. Merge fields into existing settings rather than replacing unrelated configuration. Keep authentication in the private credential store/environment, not this example. `/settings` exposes common options; restart after manually editing startup-only settings.

Other resources include `models.json` for custom model/provider configuration, `mcp.json` for server connections, `keybindings.json`, `hooks.json`, and resource directories. Inspect [settings fields](crates/davinci-coding-agent/src/settings.rs) and [project resolution](crates/davinci-coding-agent/src/project_config.rs) for exact behavior.

### Useful environment variables

`autoVerify` defaults to `true` in `settings.json`. After an edit invalidates a
previous verification, the harness repeats the last verification call through
the normal permission checks. It preserves the arguments and working directory.
Set `autoVerify` to `false`, or `DAVINCI_AUTO_VERIFY` to `0`, `false`, or `off`,
to keep the model reminder instead. The environment switch takes precedence
and also applies when settings are reloaded.

Fast mode keeps the selected model and reasoning effort on the ChatGPT/Codex
subscription route; no API key is required. Use `/fast` to toggle Fast and Standard
for the active session. The choice is saved to user settings for future sessions;
a failed save is reported as “this run only.” `/status` and the model footer show
the requested speed. Unknown model capabilities are marked unverified; an explicit
backend downgrade produces a notice without retrying the turn. Fast is available
only for OpenAI Codex models; other providers receive no service-tier field and
show no speed indicator.

In user-level `settings.json`, set `"serviceTier": "fast"`. `priority` remains an
accepted alias and `flex` remains supported. `standard`/`default` or an omitted
setting uses Standard, the default. Project settings cannot select the user's tier.
`DAVINCI_OPENAI_SERVICE_TIER=fast` overrides user settings at startup; `/fast`
overrides that input for the current session. Invalid values produce a diagnostic
and use Standard. These settings apply to interactive, print, JSON and RPC calls;
child agents inherit the host selection. Fast may consume subscription limits
faster, and availability depends on the model, backend and account.

`effortPolicy` defaults to `fixed`. Opt in with `adaptive` to request lower reasoning
effort before the turn's first successful edit, the configured effort after an edit,
and higher effort after two consecutive failed tool results (including batch children).
`off` remains off. `DAVINCI_EFFORT_POLICY` overrides the setting; unknown values use
`fixed`. The configured thinking level and system prompt identity stay unchanged.

`toolSurface` defaults to `full`. Set it to `lean` to initially send core tool schemas
on cache-sensitive routes; other authorized tools remain available through
`tool_search`. Discovery adds a schema once and keeps it visible for later requests.
Permission rules still apply. `DAVINCI_TOOL_SURFACE` overrides the setting; unknown
values use `full`. This setting applies when a new agent starts. Explicit tool
selections stay exposed, and routes without appended turn context are unchanged.

~~~text
DAVINCI_CODING_AGENT_DIR
DAVINCI_CODING_AGENT_SESSION_DIR
PI_CODING_AGENT_DIR
PI_CODING_AGENT_SESSION_DIR
PI_OFFLINE
PI_NODE
~~~

Provider-specific credentials use the provider's normal environment variables. Additional opt-ins include `DAVINCI_EXPERIMENTAL_AGENT_TEAMS`, `DAVINCI_EXPERIMENTAL_WORKFLOWS`, `DAVINCI_WORKFLOW_MAX_CONCURRENT_AGENTS`, and `DAVINCI_PLUGINS=off` to disable plugin loading. `DAVINCI_MCP_CONFIG` (legacy `PI_MCP_CONFIG`) selects an explicit MCP configuration file.

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

Enabled plugin servers form the base layer, user entries override matching names, and trusted project `.davinci/mcp.json` (or legacy `.pi/mcp.json`) entries override those. An explicit `DAVINCI_MCP_CONFIG` / `PI_MCP_CONFIG` file replaces that discovery path.

Use `/mcp` for connection/tool errors and `--no-mcp` to skip all MCP connections for a run. Local MCP processes may execute third-party code; project trust, permissions, and active sandbox policy matter. See [MCP loader](crates/davinci-coding-agent/src/mcp.rs), [davinci-mcp](crates/davinci-mcp), and [sandbox service boundaries](docs/sandbox.md).

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
| davinci-sys | Shared filesystem/platform primitives |
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
cargo test --workspace --locked
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

Some tests intentionally require external software, a browser, language server, audio stack, or live provider credentials. Offline fixture tests do not prove those external integrations. Read [AGENTS.md](AGENTS.md) before contributing; use focused checks for small changes and the relevant CI gates for shared runtime contracts. The commands here describe how to validate a checkout, not a claim that every listed test passed for the reader's installation.

Useful starting points:

- [CI workflow and exact job commands](.github/workflows/ci.yml)
- [Behavioral evaluation and live/offline distinction](docs/behavioral-evals.md)
- [Production-readiness evidence and remaining gates](docs/readiness/README.md)
- [Runtime recovery and safe retry](docs/runtime/recovery-playbook.md)

Live model evaluations consume account usage. Run them only with an explicit provider, scope, and budget; do not treat an offline fixture pass as a measured subscription saving.

---

## Documentation

Start here:

- [Architecture and navigation](docs/ARCHITECTURE.md)
- [Documentation index](docs/README.md)
- [Runtime orchestration](docs/runtime-orchestration.md)
- [Subagents, teams, and workflow controls](docs/agent-teams.md)
- [Permissions and planning](docs/permission-modes.md)
- [Execution sandbox](docs/sandbox.md)
- [Plugins and marketplaces](docs/plugins.md)
- [Design artifacts](docs/design-artifacts.md)
- [Session diagnostics](docs/session-diagnostics.md)
- [OpenAI subscription efficiency](docs/openai-efficiency.md)
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

## Troubleshooting and limitations

| Symptom | Check / next step |
| --- | --- |
| A new command is missing although `--version` says 1.0.71 | Check the PATH-resolved executable and `/doctor` installation identity. Compare its source commit with this README baseline; rebuild/restart from the intended source. |
| Release installation fails before compilation | Read the preflight error. Check a clean checkout, exact version tag, Python, authenticated `gh`, and completed green CI/lint for that commit. A development build and a verified release have different provenance. |
| `davinci update self` fails | This Rust product has no self-update package. Use the source or release installation workflow. |
| `openai-codex` reports no usable credential | Run `/login openai-codex` in the new binary and wait for a stored-login confirmation. Legacy tokens and API keys are not accepted substitutes. |
| Browser login returns an OAuth/registration/callback error | Confirm the browser and CLI can share the loopback callback, port 1455 is available, and the system clock is correct. Preserve the non-secret error. Do not treat a printed URL or `auth check` alone as a live completion test. |
| An API-key prompt appears during intended subscription use | Check `/status` and select provider `openai-codex`. Provider `openai` is a separate API-key route. Do not purchase/use API access merely to mask a subscription-login failure. |
| Model unavailable or missing from selection | Check `--list-models`, refresh with `update --models`, and confirm account entitlement. A catalog entry or cached Codex model name does not prove access. |
| `(offline) received ... characters` | Expected fixture response. Remove `--offline` and unset `PI_OFFLINE` for real provider inference. |
| Print/JSON stops for approval | Inspect `approval_required` and the chosen policy. Use an appropriate authorized mode, or `deny-continue` when partial read-only results are acceptable; do not assume the blocked action completed. |
| A shell command is denied after approval | An explicit denial or OS boundary can still block it. Inspect `/sandbox-status`; permission approval does not expand isolation. |
| A project setting, skill, hook, or server is missing | Check the working directory, trust choice, `.davinci`/legacy `.pi` paths, enabled plugin state, and hook approval. Use `/setup check`, `/doctor`, and `/mcp`. |
| A language server is unavailable | Install/configure the supported local server yourself; DaVinci does not provision it automatically. See [language intelligence](docs/language-intelligence.md). |
| Dictation is unavailable | Check `voice status`, the matching native worker, an installed/imported model, microphone permissions, and platform build dependencies. |
| Native inspector exits 3 | Evidence is unavailable/inconsistent, not an instruction to remove it. Preserve state and follow the [recovery playbook](docs/runtime/recovery-playbook.md). |

Current limits to keep in mind:

- ChatGPT login, Fast support, model access, and live task completion depend on actual backend/account behavior; offline/source verification cannot certify them.
- Agent teams, dynamic workflows, Context VM modes, design artifacts, and IPC have distinct opt-ins or build/runtime prerequisites.
- The default and legacy TUIs have different feature coverage. Terminal previews and mockup fixtures are not live integration evidence.
- Security reports, language diagnostics, and verification plans are advisory inputs. Actual verification commands, current workspace state, and retained evidence determine what was checked.
- Browser, Node, language-server, voice, and OS-sandbox support vary by platform. See the individual capability/readiness guides rather than assuming uniform support.
- There is no published prebuilt release or working Rust self-updater documented here. Rebuilding source does not automatically refresh an already running session.

For a bug report, include OS/terminal, the executable path, version and source identity if known, exact non-secret command, expected/actual behavior, and the relevant redacted error. Never attach `auth.json`, bearer tokens, API keys, or complete OAuth redirect URLs.

---

## Security notes

DaVinci can execute model-requested shell commands and can modify files when the active permission policy allows it.

Important boundaries:

- approval modes and OS execution isolation are separate; inspect the actual sandbox status;
- project-local executable/configurable resources are protected by project trust;
- credentials should never be committed to the repository;
- without a verified execution boundary, local services and child processes can run with the user's OS permissions; an active boundary constrains only its supported execution paths;
- security-scan results are evidence, not a guarantee that vulnerabilities are absent;
- `--offline` uses fixture responses and disables supported startup network work; it is neither a local inference engine nor a complete network-isolation control;
- review exported sessions and `/share` uploads for private code, personal information, and secrets before sharing.

Review [docs/security](docs/security/) before deploying DaVinci in a sensitive environment.

---

## License

DaVinci is licensed under the [MIT License](LICENSE).

The repository also contains vendored/reference components with their own licenses and notices. Local voice native dependencies and model provenance are documented under [crates/davinci-voice](crates/davinci-voice) and [docs/voice-input.md](docs/voice-input.md).
