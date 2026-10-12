//! WS4 / WS1 diagnosis: what the `openai-codex` provider is sent, request by
//! request, for one scripted conversation.
//!
//! Diagnosis only. Nothing here changes production behaviour.
//!
//! # What the harness is, and what it is not
//!
//! Print mode sends a request from the `run_loop` closure inside
//! `complete_prompt_with_host` (main.rs). That closure ends in
//! `live_complete_streaming_with_sink_envelope`, which opens a socket, so it
//! cannot be driven offline. This harness therefore:
//!
//! * builds the agent with the real `build_agent` (the print-mode builder),
//!   from a `settings.json` holding the benchmark's settings;
//! * repeats the per-prompt setup of `complete_prompt_with_host` with the same
//!   private functions (`attach_shared_tool_executor`, the runtime handle and
//!   capability registration, `freeze_tools_for_cache`, the Token Governor
//!   pre/post tool hooks, `synchronize_provider_system_prompt`, ...);
//! * runs the real `Agent::run_loop` and real tools in a real directory;
//! * inside the closure, builds the request with the same calls and the same
//!   `StreamOptions` fields as the production closure (`provider_tools`,
//!   `live_cache_key`, `messages_for_provider`, `native_responses_resume_record_for`,
//!   `davinci_ai::request_body_with`), then applies what
//!   `live_complete_streaming_with_sink_envelope_inner` applies before the
//!   send (`responses_is_oauth`, `stream: true`, `PreparedProviderRequest`)
//!   and serializes as `stream_http::send` does (`Value::to_string`);
//! * answers with a scripted Responses SSE stream decoded by the real decoder
//!   (`davinci_ai::fixture_complete`), including reasoning items with
//!   `encrypted_content` and native output items.
//!
//! Each run happens in a child process (this test binary re-invoked on
//! `ws_child_run`): with a session, the operation journal under
//! `<cwd>/.davinci/operations` stays open for the life of the process, so a
//! second run in the same process could neither clean the directory nor start
//! from the same state. One process per run also matches print mode.
//!
//! Not covered: HTTP headers, user hooks/plugins, MCP, extensions, memory
//! injection, context files and skills (all disabled or absent here), and the
//! production closure itself, which is replicated rather than called.
#![allow(clippy::too_many_lines)]

use super::*;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

const WINDOW_LUNA: u64 = 272_000;

/// What the scripted model does in one response.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
enum StepKind {
    /// Not a scripted reply: the first request after a user prompt.
    UserPrompt,
    Ls,
    PlainRead,
    ParallelReads,
    FailedRead,
    /// Same file again inside the governor's dedupe window.
    RepeatedRead,
    /// More than 200 lines of `ls` output: the governor compresses it.
    LargeLs,
    Write,
    Edit,
    /// A big distinct read, used to fill the context in the compaction run.
    BigRead,
    FinalText,
    /// The script ran out; the model answers a harness follow-up in text.
    Followup,
}

#[derive(Debug, Clone)]
struct Step {
    kind: StepKind,
    calls: Vec<(&'static str, Value)>,
    text: Option<String>,
}

fn call(kind: StepKind, name: &'static str, arguments: Value) -> Step {
    Step {
        kind,
        calls: vec![(name, arguments)],
        text: None,
    }
}

fn final_text(text: &str) -> Step {
    Step {
        kind: StepKind::FinalText,
        calls: Vec::new(),
        text: Some(text.into()),
    }
}

/// Deterministic workspace: the same bytes for every run.
fn seed_workspace(cwd: &Path) {
    std::fs::create_dir_all(cwd.join("src")).unwrap();
    std::fs::create_dir_all(cwd.join("notes")).unwrap();
    std::fs::create_dir_all(cwd.join("data/wide")).unwrap();
    std::fs::create_dir_all(cwd.join("data/big")).unwrap();
    std::fs::write(
        cwd.join("README.md"),
        "# fixture\nScheduler crate fixture.\n",
    )
    .unwrap();
    std::fs::write(
        cwd.join("src/lib.rs"),
        (0..30)
            .map(|n| format!("pub fn lib_{n}() -> u32 {{ {n} }}\n"))
            .collect::<String>(),
    )
    .unwrap();
    for name in ["a", "b"] {
        std::fs::write(
            cwd.join(format!("src/{name}.rs")),
            (0..20)
                .map(|n| format!("pub fn {name}_{n}() -> u32 {{ {n} }}\n"))
                .collect::<String>(),
        )
        .unwrap();
    }
    for index in 0..40 {
        std::fs::write(
            cwd.join(format!("src/m{index:02}.rs")),
            (0..12)
                .map(|n| format!("pub fn m{index:02}_{n}(x: u32) -> u32 {{ x + {n} }}\n"))
                .collect::<String>(),
        )
        .unwrap();
    }
    for index in 0..320 {
        std::fs::write(cwd.join(format!("data/wide/entry_{index:03}.txt")), "x\n").unwrap();
    }
    // About 12 KB each, distinct content: 3k tokens of tool output per read.
    for index in 0..80 {
        std::fs::write(
            cwd.join(format!("data/big/big_{index:02}.rs")),
            (0..150)
                .map(|n| {
                    format!(
                        "pub fn big_{index:02}_{n:03}(job: &Job, retries: u32) -> Result<u32, Error> {{ todo!() }}\n"
                    )
                })
                .collect::<String>(),
        )
        .unwrap();
    }
}

/// The equivalence conversation: 2 prompts, more than 40 requests, every step
/// kind several times, far below any pruning or compaction line at 272k.
fn equivalence_script(root_listing: bool) -> Vec<(String, Vec<Step>)> {
    use StepKind::*;
    let mut first = vec![
        // With a session the workspace root holds `.davinci/` (the operation
        // journal), which `ls .` shows the model. `root_listing: false` keeps
        // that one known difference out of a session/no-session comparison.
        call(
            Ls,
            "ls",
            json!({"path": if root_listing { "." } else { "notes" }}),
        ),
        call(PlainRead, "read", json!({"path": "src/lib.rs"})),
        Step {
            kind: ParallelReads,
            calls: vec![
                ("read", json!({"path": "src/a.rs"})),
                ("read", json!({"path": "src/b.rs"})),
            ],
            text: None,
        },
        call(FailedRead, "read", json!({"path": "src/missing.rs"})),
        call(RepeatedRead, "read", json!({"path": "src/lib.rs"})),
        call(LargeLs, "ls", json!({"path": "data/wide", "limit": 1000})),
        call(
            Write,
            "write",
            json!({"path": "notes/n1.txt", "content": "first note\n"}),
        ),
        call(
            Edit,
            "edit",
            json!({"path": "src/a.rs", "edits": [{"oldText": "pub fn a_0() -> u32 { 0 }", "newText": "pub fn a_0() -> u32 { 100 }"}]}),
        ),
    ];
    for index in 0..24 {
        let path = format!("src/m{index:02}.rs");
        first.push(match index % 6 {
            0 => call(PlainRead, "read", json!({"path": path})),
            1 => Step {
                kind: ParallelReads,
                calls: vec![
                    ("read", json!({"path": path})),
                    ("ls", json!({"path": "src"})),
                ],
                text: None,
            },
            2 => call(
                FailedRead,
                "read",
                json!({"path": format!("src/nope{index}.rs")}),
            ),
            3 => {
                // The file read three steps earlier (index - 3 is the `0` arm).
                call(
                    RepeatedRead,
                    "read",
                    json!({"path": format!("src/m{:02}.rs", index - 3)}),
                )
            }
            4 => call(
                Write,
                "write",
                json!({"path": format!("notes/w{index}.txt"), "content": format!("note {index}\n")}),
            ),
            _ => call(LargeLs, "ls", json!({"path": "data/wide", "limit": 1000})),
        });
    }
    first.push(final_text("Part one is done."));
    let second = vec![
        call(PlainRead, "read", json!({"path": "README.md"})),
        call(
            Edit,
            "edit",
            json!({"path": "src/b.rs", "edits": [{"oldText": "pub fn b_0() -> u32 { 0 }", "newText": "pub fn b_0() -> u32 { 200 }"}]}),
        ),
        call(RepeatedRead, "read", json!({"path": "README.md"})),
        Step {
            kind: ParallelReads,
            calls: vec![
                ("read", json!({"path": "src/m30.rs"})),
                ("read", json!({"path": "src/m31.rs"})),
            ],
            text: None,
        },
        call(FailedRead, "read", json!({"path": "src/absent.rs"})),
        final_text("Part two is done."),
    ];
    vec![
        (
            "Inspect the scheduler crate, note what you find in notes/, and fix a_0. Never modify files under migrations/.".into(),
            first,
        ),
        ("Now fix b_0 as well and re-check the README.".into(), second),
    ]
}

/// Fills the context with distinct 12 KB reads until something gives.
fn filling_script(reads: usize) -> Vec<(String, Vec<Step>)> {
    let mut steps: Vec<Step> = (0..reads)
        .map(|index| {
            call(
                StepKind::BigRead,
                "read",
                json!({"path": format!("data/big/big_{index:02}.rs")}),
            )
        })
        .collect();
    steps.push(final_text("Read everything."));
    vec![(
        "Read every file under data/big. Never modify files under migrations/.".into(),
        steps,
    )]
}

/// 100 scripted model turns of small reads, for the timing comparison.
fn timing_script() -> Vec<(String, Vec<Step>)> {
    let mut steps: Vec<Step> = (0..99)
        .map(|index| {
            call(
                StepKind::PlainRead,
                "read",
                json!({"path": format!("src/m{:02}.rs", index % 40), "offset": 1 + index / 40, "limit": 6}),
            )
        })
        .collect();
    steps.push(final_text("Timing run complete."));
    vec![("Read the module files one at a time.".into(), steps)]
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Captured {
    prompt: usize,
    /// The scripted step whose output precedes this request (what was
    /// appended since the previous request).
    after: StepKind,
    /// The request body parsed back from `wire`, filled in by the parent.
    #[serde(skip)]
    body: Value,
    /// The serialized request exactly as `stream_http::send` would post it.
    wire: String,
    compactions: u64,
    pruned_results: u64,
    native_replay: bool,
    transport_session: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct ToolResultSeen {
    call_id: String,
    tool: String,
    is_error: Option<bool>,
    text: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct RunResult {
    requests: Vec<Captured>,
    tool_results: Vec<ToolResultSeen>,
    governor: Value,
    compactions: u64,
    pruned_results: u64,
    /// run_loop wall time minus time spent inside the provider closure, ms.
    harness_ms: f64,
    /// Time spent building and serializing requests inside the closure, ms.
    request_build_ms: f64,
    harness_messages: Vec<String>,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
enum Script {
    Equivalence,
    /// The same conversation without the listing of the workspace root.
    EquivalenceNoRootListing,
    Filling(usize),
    Timing,
}

impl Script {
    fn steps(self) -> Vec<(String, Vec<Step>)> {
        match self {
            Script::Equivalence => equivalence_script(true),
            Script::EquivalenceNoRootListing => equivalence_script(false),
            Script::Filling(reads) => filling_script(reads),
            Script::Timing => timing_script(),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct RunConfig {
    /// Names the fixed working directory: runs compared with each other must
    /// share it so model-visible paths are identical.
    label: String,
    mode: String,
    session: bool,
    context_window: u64,
    script: Script,
    /// `DAVINCI_PRUNE_PROFILE` for the run (`None`: unset, the default).
    prune_profile: Option<String>,
    /// Build each request twice and require identical bytes.
    check_determinism: bool,
}

fn config(label: &str, mode: &str, session: bool) -> RunConfig {
    RunConfig {
        label: label.into(),
        mode: mode.into(),
        session,
        context_window: WINDOW_LUNA,
        script: Script::Equivalence,
        prune_profile: None,
        check_determinism: false,
    }
}

fn codex_model() -> davinci_ai::Model {
    let mut model = davinci_ai::load_builtin_models()
        .into_iter()
        .find(|model| model.provider == "openai-codex" && model.id == "gpt-5.6-luna")
        .expect("catalog has a Codex Luna template");
    model.id = "gpt-6-luna".into();
    assert!(
        davinci_ai::openai_siwc::is_public_plan_model(&model),
        "the benchmark route is the public Sign in with ChatGPT plan route"
    );
    model
}

fn oauth() -> ResolvedAuth {
    ResolvedAuth {
        api_key: Some("test".into()),
        headers: Default::default(),
        source: "oauth".into(),
    }
}

/// A scripted Responses stream for one step, decoded by the real decoder.
fn scripted_reply(
    model: &davinci_ai::Model,
    messages: &[davinci_ai::ChatMessage],
    index: usize,
    step: &Step,
    input_tokens: u64,
) -> (AssistantMessage, Value) {
    let mut items = vec![json!({
        "type": "reasoning",
        "id": format!("rs_{index:04}"),
        "summary": [{"type": "summary_text", "text": format!("**Step {index}**")}],
        "encrypted_content": format!("gAAAAfixture{index:04}{}", "Zm9v".repeat(24)),
    })];
    if let Some(text) = &step.text {
        items.push(json!({
            "type": "message",
            "id": format!("msg_{index:04}"),
            "role": "assistant",
            "status": "completed",
            "content": [{"type": "output_text", "text": text, "annotations": []}],
        }));
    }
    for (position, (name, arguments)) in step.calls.iter().enumerate() {
        items.push(json!({
            "type": "function_call",
            "id": format!("fc_{index:04}_{position}"),
            "call_id": format!("call_{index:04}_{position}"),
            "name": name,
            "arguments": arguments.to_string(),
            "status": "completed",
        }));
    }
    let mut corpus = format!(
        "data: {}\n\n",
        json!({"type": "response.created", "response": {"id": format!("resp_{index:04}")}})
    );
    for (output_index, item) in items.iter().enumerate() {
        let mut added = item.clone();
        match item["type"].as_str() {
            Some("function_call") => added["arguments"] = json!(""),
            Some("message") => added["content"] = json!([]),
            Some("reasoning") => added["summary"] = json!([]),
            _ => {}
        }
        corpus.push_str(&format!(
            "data: {}\n\n",
            json!({"type": "response.output_item.added", "output_index": output_index, "item": added})
        ));
        match item["type"].as_str() {
            Some("function_call") => corpus.push_str(&format!(
                "data: {}\n\n",
                json!({"type": "response.function_call_arguments.done", "output_index": output_index, "arguments": item["arguments"]})
            )),
            Some("message") => corpus.push_str(&format!(
                "data: {}\n\n",
                json!({"type": "response.output_text.delta", "output_index": output_index, "delta": item["content"][0]["text"]})
            )),
            Some("reasoning") => corpus.push_str(&format!(
                "data: {}\n\n",
                json!({"type": "response.reasoning_summary_text.delta", "output_index": output_index, "delta": item["summary"][0]["text"]})
            )),
            _ => {}
        }
        corpus.push_str(&format!(
            "data: {}\n\n",
            json!({"type": "response.output_item.done", "output_index": output_index, "item": item})
        ));
    }
    let completed = json!({"type": "response.completed", "response": {
        "id": format!("resp_{index:04}"),
        "status": "completed",
        "output": items,
        "usage": {
            "input_tokens": input_tokens,
            "output_tokens": 40,
            "total_tokens": input_tokens + 40,
            "input_tokens_details": {"cached_tokens": 0},
        },
    }});
    corpus.push_str(&format!("data: {completed}\n\n"));
    (
        davinci_ai::fixture_complete(model, messages, &corpus),
        completed,
    )
}

/// Removal must succeed: a leftover would be visible to `ls` in the next run.
fn remove_tree(root: &Path) {
    let mut last = None;
    for _ in 0..50 {
        match std::fs::remove_dir_all(root) {
            Ok(()) => return,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => last = Some(error),
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("cannot remove {}: {:?}", root.display(), last);
}

const CHILD_CONFIG: &str = "WS_WIRE_CHILD_CONFIG";
const CHILD_OUTPUT: &str = "WS_WIRE_CHILD_OUTPUT";
const CHILD_ROOT: &str = "WS_WIRE_CHILD_ROOT";

/// Run one conversation in a fresh process and read back what it sent.
fn run(config: &RunConfig) -> RunResult {
    // One fixed directory per label; runs under one label are sequential.
    let root = std::env::temp_dir().join(format!(
        "davinci-ws-wire-{}-{}",
        std::process::id(),
        config.label
    ));
    remove_tree(&root);
    std::fs::create_dir_all(&root).unwrap();
    let output = root.join("result.json");
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    // Other tests in this binary set harness variables on the shared process
    // environment while they run, and a child would inherit whatever is set
    // at the moment it starts. Drop them all before setting this run's own.
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy().to_ascii_uppercase();
        if name.starts_with("DAVINCI_") || name.starts_with("PI_") {
            command.env_remove(&key);
        }
    }
    command
        .args([
            "--exact",
            "ws_wire_diagnosis_tests::ws_child_run",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_CONFIG, serde_json::to_string(config).unwrap())
        .env(CHILD_OUTPUT, &output)
        .env(CHILD_ROOT, &root)
        .env("PI_CODING_AGENT_DIR", root.join("agent"));
    if let Some(profile) = &config.prune_profile {
        command.env("DAVINCI_PRUNE_PROFILE", profile);
    }
    let finished = command.output().expect("child test process starts");
    assert!(
        finished.status.success() && output.is_file(),
        "child run {config:?} failed:\n{}\n{}",
        String::from_utf8_lossy(&finished.stdout),
        String::from_utf8_lossy(&finished.stderr)
    );
    let mut result: RunResult =
        serde_json::from_str(&std::fs::read_to_string(&output).unwrap()).unwrap();
    for request in &mut result.requests {
        request.body = serde_json::from_str(&request.wire).unwrap();
    }
    remove_tree(&root);
    result
}

/// The child half of [`run`]. Does nothing unless a parent test asked for it.
#[test]
#[ignore = "child process of the ws4_/ws1_ tests; not a test on its own"]
fn ws_child_run() {
    let (Ok(config), Ok(output), Ok(root)) = (
        std::env::var(CHILD_CONFIG),
        std::env::var(CHILD_OUTPUT),
        std::env::var(CHILD_ROOT),
    ) else {
        return;
    };
    let config: RunConfig = serde_json::from_str(&config).unwrap();
    let result = run_in_process(&config, Path::new(&root));
    std::fs::write(output, serde_json::to_string(&result).unwrap()).unwrap();
}

fn run_in_process(config: &RunConfig, root: &Path) -> RunResult {
    let agent_dir = root.join("agent");
    let cwd = root.join("work");
    let sessions = root.join("sessions");
    for dir in [&agent_dir, &cwd, &sessions] {
        std::fs::create_dir_all(dir).unwrap();
    }
    seed_workspace(&cwd);
    // The benchmark's settings (C:\bench\luna-high-wire-settings), with the
    // Context VM mode under test.
    std::fs::write(
        agent_dir.join("settings.json"),
        serde_json::to_string_pretty(&json!({
            "compaction": {"enabled": true, "reserveTokens": 16384, "keepRecentTokens": 20000},
            "showCacheMissNotices": false,
            "transport": "auto",
            "serviceTier": null,
            "effortPolicy": "fixed",
            "toolSurface": "full",
            "autoVerify": true,
            "promptProfile": "stable",
            "contextVm": config.mode.as_str(),
        }))
        .unwrap(),
    )
    .unwrap();
    // The benchmark command line: --mode json [--no-session] --provider
    // openai-codex --model gpt-6-luna --thinking high --permission-mode
    // always-approve -p <prompt>. MCP, extensions, skills and context files
    // are off so nothing outside the fixture reaches the request.
    let parsed = Args {
        mode: Some(Mode::Json),
        provider: Some("openai-codex".into()),
        model: Some("gpt-6-luna".into()),
        thinking: Some(davinci_protocol::ThinkingLevel::High),
        permission_mode: Some(davinci_agent::PermissionMode::AlwaysApprove),
        no_session: !config.session,
        no_mcp: true,
        no_extensions: true,
        no_skills: true,
        no_context_files: true,
        ..Default::default()
    };
    let mut agent = build_agent(&parsed, &sessions, &cwd).expect("print-mode agent");
    let model = codex_model();
    let auth = oauth();
    agent.provider = model.provider.clone();
    agent.model_id = model.id.clone();
    agent.context_window = config.context_window;
    agent.retry_attempts = 0;
    assert_eq!(agent.session.is_some(), config.session);
    assert_eq!(
        agent.context_vm_mode(),
        davinci_agent::runtime::ContextVmMode::from_env_value(Some(config.mode.as_str())),
        "settings.json contextVm reached the agent"
    );
    assert_eq!(
        agent.turn_context_placement(),
        davinci_agent::turn_context::TurnContextPlacement::Appended,
        "openai-codex is a cache-sensitive route"
    );
    // build_agent installs a live summarizer (a provider call). A fixture
    // that proposes nothing new keeps compaction offline and deterministic.
    agent.summarizer = Some(Summarizer::new(|request| {
        Ok(davinci_agent::SummarizeResponse {
            text: if request.label == "context state fold" {
                "{}".into()
            } else {
                "## Goal\nContinue the fixture task.".into()
            },
            usage: Default::default(),
            stop_reason: Some(StopReason::Stop),
            error_message: None,
            has_tool_call: false,
        })
    }));

    let host = Arc::new(Mutex::new(loaded_extension_host_for_cwd(&parsed, &cwd)));
    let mut requests: Vec<Captured> = Vec::new();
    let mut harness_time = Duration::ZERO;
    let mut request_build_time = Duration::ZERO;
    let mut reply_index = 0usize;

    let script = config.script.steps();
    for (prompt_index, (prompt, steps)) in script.iter().enumerate() {
        // run_print_turns
        agent.prompt_user_with(prompt, &[]);
        // complete_prompt_with_host, in its order.
        attach_shared_tool_executor(&mut agent, host.clone());
        agent.clear_ephemeral_context();
        agent.reset_system_prompt_to_base();
        agent.commit_turn_context(None);
        let runtime_bus = davinci_agent::RuntimeBus::new();
        runtime_bus.subscribe(Arc::new(runtime_host::CompactionRuntimeSubscriber::new(
            host.lock()
                .unwrap_or_else(|error| error.into_inner())
                .native
                .clone(),
        )));
        let mut runtime_handle = davinci_agent::RuntimeHandle::new(
            davinci_agent::RunId::new(),
            davinci_agent::AgentId::new(),
            runtime_bus.clone(),
        )
        .with_cache(agent.tool_context.cache.clone())
        .with_project_trusted(false);
        let wf_store = davinci_agent::WorkflowStateStore::with_options(
            davinci_agent::runtime::workflow::state::DEFAULT_MAX_INLINE_ARTIFACT_BYTES,
            agent_dir.join("workflow_artifacts"),
        );
        runtime_handle = runtime_host::configure_session_workflow(
            runtime_handle,
            agent.session.as_ref(),
            agent.runtime_for_next_prompt(),
            wf_store,
            agent.subagent_runner.clone(),
        )
        .and_then(|runtime| {
            runtime_host::attach_operation_runtime(runtime, &agent.cwd, agent.session.as_ref())
        })
        .expect("runtime");
        host.lock()
            .unwrap_or_else(|error| error.into_inner())
            .register_with(&runtime_handle.capability_registry);
        agent.set_runtime(runtime_handle);
        agent.activate_staged_codemode().expect("codemode");
        agent.freeze_tools_for_cache();
        apply_graph_session_context(
            &parsed,
            &agent,
            &host.lock().unwrap_or_else(|error| error.into_inner()),
        );
        bind_test_impact_context(
            &agent,
            &host.lock().unwrap_or_else(|error| error.into_inner()),
        );
        // The Token Governor's two hooks, as the production closures call them.
        let pre_host = host.clone();
        agent.pre_tool = Some(davinci_agent::PreToolHook(Arc::new(move |name, args| {
            pre_host
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .native_before_tool(name, args, String::new)
        })));
        let post_host = host.clone();
        agent.post_tool = Some(davinci_agent::PostToolHook(Arc::new(
            move |_id, _cwd, name, args, result| match post_host.lock() {
                Ok(host) => host.native_after_tool(name, args, result),
                Err(_) => result,
            },
        )));
        synchronize_provider_system_prompt(&mut agent);
        agent.set_provider_context_overhead_estimator(provider_tool_overhead_tokens);
        agent.set_provider_output_limit(Some(model.max_tokens));
        let mut context_visibility = (agent.stats.pruned_results, agent.stats.compactions);
        let transport_session = ProviderTransportSession::new(&agent);

        let mut step_cursor = 0usize;
        let mut after = StepKind::UserPrompt;
        let mut closure_time = Duration::ZERO;
        let loop_started = Instant::now();
        let outcome = agent.run_loop(|current: &Agent| {
            let closure_started = Instant::now();
            let visibility = (current.stats.pruned_results, current.stats.compactions);
            if visibility != context_visibility {
                host.lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .native_context_pruned();
                context_visibility = visibility;
            }
            let system = current.provider_system_prompt();
            let provider_messages = current.messages_for_provider();
            let request_tools = provider_tools(current);
            let cache_key = Some(live_cache_key(
                &model,
                &auth,
                &system,
                current,
                &request_tools,
            ));
            let options = StreamOptions {
                thinking_level: Some(current.request_thinking_level()),
                service_tier: Some(current.service_tier),
                thinking_budgets: current.thinking_budgets.clone(),
                // `options_with_auth_context` in davinci-ai sets this from the
                // credential source before the body is built.
                responses_is_oauth: Some(true),
                timeout_ms: current.provider_timeout_ms,
                max_retries: current.provider_max_retries,
                max_retry_delay_ms: Some(current.provider_max_retry_delay_ms),
                max_tokens: current.context_vm_provider_output_limit(),
                websocket_connect_timeout_ms: None,
                transport: current.transport.clone(),
                session_id: Some(transport_session.id.clone()),
                cache_key,
                cache_retention: None,
                native_responses_resume: current
                    .native_responses_resume_record_for(&provider_messages),
                install_telemetry: Some(current.install_telemetry),
                abort_signal: None,
                output_schema: current.output_schema.clone(),
            };
            let build = |options: &StreamOptions| {
                let mut body = davinci_ai::request_body_with(
                    &model,
                    &provider_messages,
                    Some(&system),
                    &request_tools,
                    options,
                );
                // live_complete_streaming_with_sink_envelope_inner
                body["stream"] = Value::Bool(true);
                davinci_ai::responses_request::PreparedProviderRequest::new(body)
            };
            let prepared = build(&options);
            // stream_http::send: `body.to_string().into_bytes()`
            let bytes = prepared.body().to_string();
            if config.check_determinism {
                let again = build(&options).body().to_string();
                assert!(
                    bytes == again,
                    "request {} serialized differently on a second build",
                    requests.len()
                );
                let rebuilt_tools = provider_tools(current);
                assert_eq!(
                    serde_json::to_string(&rebuilt_tools).unwrap(),
                    serde_json::to_string(&request_tools).unwrap(),
                    "tool specs changed between two calls"
                );
            }
            request_build_time += closure_started.elapsed();
            requests.push(Captured {
                prompt: prompt_index,
                after,
                body: Value::Null,
                wire: bytes.clone(),
                compactions: current.stats.compactions,
                pruned_results: current.stats.pruned_results,
                native_replay: options.native_responses_resume.is_some(),
                transport_session: transport_session.id.clone(),
            });
            let followup;
            let step = match steps.get(step_cursor) {
                Some(step) => step,
                None => {
                    followup = Step {
                        kind: StepKind::Followup,
                        calls: Vec::new(),
                        text: Some("Verified; nothing further.".into()),
                    };
                    &followup
                }
            };
            step_cursor += 1;
            after = step.kind;
            let (message, completed) = scripted_reply(
                &model,
                &provider_messages,
                reply_index,
                step,
                bytes.len() as u64 / 4,
            );
            reply_index += 1;
            let native_responses_resume =
                davinci_ai::responses_ledger::NativeResponsesOutput::from_response_value(
                    &completed,
                )
                .and_then(|output| {
                    davinci_ai::responses_ledger::NativeResponsesTurn::from_prepared(
                        &prepared, output,
                    )
                })
                .map(|turn| {
                    let mut projection = provider_messages.clone();
                    projection.push(davinci_ai::assistant_to_chat(&message));
                    davinci_ai::NativeResponsesResumeRecord {
                        turn,
                        resume_provider_message_count: projection.len(),
                        resume_provider_messages_fingerprint:
                            davinci_ai::provider_messages_fingerprint(&projection),
                    }
                });
            closure_time += closure_started.elapsed();
            Ok::<_, String>(CompleteOutput {
                message,
                stream_events: None,
                native_responses_resume,
                streamed_live: false,
            })
        });
        harness_time += loop_started.elapsed().saturating_sub(closure_time);
        outcome.expect("scripted loop runs");
        assert!(
            step_cursor >= steps.len(),
            "prompt {prompt_index}: the loop stopped at step {step_cursor} of {}",
            steps.len()
        );
        agent.pre_tool = None;
        agent.post_tool = None;
    }

    let tool_results = agent
        .messages
        .iter()
        .filter(|message| message.role == "toolResult")
        .map(|message| ToolResultSeen {
            call_id: message.tool_call_id.clone().unwrap_or_default(),
            tool: message.tool_name.clone().unwrap_or_default(),
            is_error: message.is_error,
            text: content_text(&message.content),
        })
        .collect();
    let harness_messages = agent
        .messages
        .iter()
        .filter(|message| {
            message.role != "toolResult"
                && message.role != "assistant"
                && (message.extra.contains_key("davinciCapabilityReminder")
                    || message
                        .extra
                        .contains_key(davinci_agent::COMPLETION_REMINDER_FIELD)
                    || message.role == "custom")
        })
        .map(|message| {
            let text = content_text(&message.content);
            format!(
                "{}:{}",
                message.role,
                text.chars().take(60).collect::<String>().replace('\n', " ")
            )
        })
        .collect();
    let governor = host
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .native
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .governor
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .stats();
    let stats = agent.run_stats();
    drop(agent);
    drop(host);
    RunResult {
        requests,
        tool_results,
        governor: serde_json::to_value(governor).unwrap(),
        compactions: stats.compactions,
        pruned_results: stats.pruned_results,
        harness_ms: harness_time.as_secs_f64() * 1000.0,
        request_build_ms: request_build_time.as_secs_f64() * 1000.0,
        harness_messages,
    }
}

/// First place two JSON values differ: path, left, right.
fn first_diff(left: &Value, right: &Value, path: &str) -> Option<(String, Value, Value)> {
    match (left, right) {
        (Value::Object(a), Value::Object(b)) => {
            let mut keys: Vec<&String> = a.keys().chain(b.keys()).collect();
            keys.sort();
            keys.dedup();
            for key in keys {
                let child = format!("{path}.{key}");
                match (a.get(key), b.get(key)) {
                    (Some(x), Some(y)) => {
                        if let Some(found) = first_diff(x, y, &child) {
                            return Some(found);
                        }
                    }
                    (x, y) => {
                        return Some((
                            child,
                            x.cloned().unwrap_or(json!("<absent>")),
                            y.cloned().unwrap_or(json!("<absent>")),
                        ))
                    }
                }
            }
            None
        }
        (Value::Array(a), Value::Array(b)) => {
            for (index, (x, y)) in a.iter().zip(b).enumerate() {
                if let Some(found) = first_diff(x, y, &format!("{path}[{index}]")) {
                    return Some(found);
                }
            }
            (a.len() != b.len()).then(|| (format!("{path}.length"), json!(a.len()), json!(b.len())))
        }
        _ => (left != right).then(|| (path.to_string(), left.clone(), right.clone())),
    }
}

fn clip(value: &Value) -> String {
    let text = match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    if text.chars().count() > 400 {
        format!(
            "{}...<{} chars>",
            text.chars().take(400).collect::<String>(),
            text.chars().count()
        )
    } else {
        text
    }
}

/// The first request index where two runs differ, with the JSON path.
fn first_divergence(left: &RunResult, right: &RunResult) -> Option<String> {
    for (index, (a, b)) in left.requests.iter().zip(&right.requests).enumerate() {
        if a.wire != b.wire {
            let (path, x, y) = first_diff(&a.body, &b.body, "$")
                .unwrap_or_else(|| ("<bytes only>".into(), Value::Null, Value::Null));
            return Some(format!(
                "request {index} (0-based; prompt {}, after {:?}) differs at {path}\n  left : {}\n  right: {}",
                a.prompt,
                a.after,
                clip(&x),
                clip(&y)
            ));
        }
    }
    (left.requests.len() != right.requests.len()).then(|| {
        format!(
            "request counts differ: {} vs {}",
            left.requests.len(),
            right.requests.len()
        )
    })
}

fn divergence_index(left: &RunResult, right: &RunResult) -> Option<usize> {
    left.requests
        .iter()
        .zip(&right.requests)
        .position(|(a, b)| a.wire != b.wire)
        .or_else(|| {
            (left.requests.len() != right.requests.len())
                .then(|| left.requests.len().min(right.requests.len()))
        })
}

/// Instructions, tool definitions and every input item, as bytes in wire
/// order of dependency: what a prefix cache could reuse from this request.
fn cacheable_prefix(body: &Value) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend(body["tools"].to_string().into_bytes());
    bytes.push(b'\n');
    bytes.extend(body["instructions"].to_string().into_bytes());
    bytes.push(b'\n');
    for item in body["input"].as_array().expect("input array") {
        bytes.extend(item.to_string().into_bytes());
        bytes.push(b'\n');
    }
    bytes
}

/// Why request `next` does not extend request `previous`, if it does not.
fn append_violation(previous: &Captured, next: &Captured) -> Option<String> {
    if cacheable_prefix(&next.body).starts_with(&cacheable_prefix(&previous.body)) {
        return None;
    }
    for key in ["tools", "instructions"] {
        if let Some((path, x, y)) =
            first_diff(&previous.body[key], &next.body[key], &format!("$.{key}"))
        {
            return Some(format!("{path}: {} -> {}", clip(&x), clip(&y)));
        }
    }
    let before = previous.body["input"].as_array().unwrap();
    let after = next.body["input"].as_array().unwrap();
    for (index, (x, y)) in before.iter().zip(after).enumerate() {
        if let Some((path, x, y)) = first_diff(x, y, &format!("$.input[{index}]")) {
            return Some(format!("{path}: {} -> {}", clip(&x), clip(&y)));
        }
    }
    Some(format!(
        "$.input.length: {} -> {} (the request got shorter)",
        before.len(),
        after.len()
    ))
}

fn summarize(label: &str, result: &RunResult) {
    let last = result.requests.last().unwrap();
    println!(
        "WS_RUN {label}: requests={} final_input_items={} final_bytes={} compactions={} pruned_results={} native_replay_requests={} governor={} harness_messages={:?}",
        result.requests.len(),
        last.body["input"].as_array().map(Vec::len).unwrap_or(0),
        last.wire.len(),
        result.compactions,
        result.pruned_results,
        result.requests.iter().filter(|request| request.native_replay).count(),
        result.governor,
        result.harness_messages,
    );
}

// ---------------------------------------------------------------------------
// Part 1: hybrid against off
// ---------------------------------------------------------------------------

/// Q4.1 / Q4.2. One conversation, `off` and `hybrid`, with and without a
/// session: the serialized request must be the same bytes at every request.
#[test]
fn ws4_hybrid_matches_off_on_the_wire_before_compaction() {
    for session in [false, true] {
        let off = run(&config("equiv", "off", session));
        let hybrid = run(&config("equiv", "hybrid", session));
        summarize(&format!("off session={session}"), &off);
        summarize(&format!("hybrid session={session}"), &hybrid);
        assert!(
            off.requests.len() >= 40,
            "the conversation must have at least 40 requests, got {}",
            off.requests.len()
        );
        assert_eq!(off.compactions, 0, "off compacted");
        assert_eq!(hybrid.compactions, 0, "hybrid compacted");
        assert_eq!(off.pruned_results, 0, "off pruned");
        // The script reached the governor paths it was written for.
        assert!(
            off.governor["deduplicatedReads"].as_u64().unwrap_or(0) >= 1,
            "no read was deduplicated: {}",
            off.governor
        );
        assert!(
            off.governor["compressedOutputs"].as_u64().unwrap_or(0) >= 1,
            "no output was compressed: {}",
            off.governor
        );
        assert!(
            off.tool_results
                .iter()
                .any(|result| result.is_error == Some(true)),
            "no failed tool call"
        );
        let divergence = first_divergence(&off, &hybrid);
        assert!(
            divergence.is_none(),
            "session={session}: hybrid is not byte-identical to off:\n{}",
            divergence.unwrap_or_default()
        );
        // Session and no-session never share a native replay path.
        assert_eq!(
            off.requests.iter().any(|request| request.native_replay),
            session,
            "native Responses replay is used exactly when a session exists"
        );
    }
}

/// Q4.2, second half: does having a session change the request at all?
///
/// CURRENT BEHAVIOUR (documented, not forced green): yes, in one place that
/// has nothing to do with the Context VM. With a session,
/// `runtime_host::attach_operation_runtime` opens the operation journal at
/// `<cwd>/.davinci/operations`, so the workspace root gains a `.davinci/`
/// directory before the first tool runs, and a listing of the root shows it
/// to the model. `--no-session` returns early there and creates nothing.
/// Apart from that listing the two send the same bytes, although the session
/// path replays native Responses items and the other re-projects messages.
#[test]
fn ws4_session_and_no_session_send_the_same_request() {
    for mode in ["off", "hybrid"] {
        // (a) The conversation that lists the workspace root.
        let without = run(&config("session", mode, false));
        let with = run(&config("session", mode, true));
        let index = divergence_index(&without, &with);
        let detail = first_divergence(&without, &with).unwrap_or_else(|| "none".into());
        println!("WS_SESSION {mode}: no-session vs session, root listed: {detail}");
        let index = index.expect(
            "CURRENT BEHAVIOUR changed: session and --no-session requests are now identical even with a root listing",
        );
        assert_eq!(
            without.requests[index].after,
            StepKind::Ls,
            "the first difference follows the root listing"
        );
        let (path, left, right) = first_diff(
            &without.requests[index].body,
            &with.requests[index].body,
            "$",
        )
        .unwrap();
        assert!(
            path.ends_with(".output"),
            "the difference is a tool output, got {path}"
        );
        assert!(!left.as_str().unwrap_or_default().contains(".davinci/"));
        assert!(right.as_str().unwrap_or_default().contains(".davinci/"));

        // (b) The same conversation without that listing: identical bytes.
        let without = run(&RunConfig {
            script: Script::EquivalenceNoRootListing,
            ..config("session", mode, false)
        });
        let with = run(&RunConfig {
            script: Script::EquivalenceNoRootListing,
            ..config("session", mode, true)
        });
        let divergence = first_divergence(&without, &with);
        println!(
            "WS_SESSION {mode}: no-session vs session, root not listed: {}",
            divergence.as_deref().unwrap_or("none")
        );
        assert!(
            divergence.is_none(),
            "{mode}: --no-session and session requests differ beyond the root listing:\n{}",
            divergence.unwrap_or_default()
        );
        assert!(with
            .requests
            .iter()
            .skip(1)
            .any(|request| request.native_replay));
        assert!(!without.requests.iter().any(|request| request.native_replay));
        // The transport session id differs (fresh UUID without a session),
        // and reaches neither run's body.
        assert_ne!(
            without.requests[0].transport_session,
            with.requests[0].transport_session
        );
    }
}

/// Q4.4. What the model is shown as tool output, and which tools it is
/// offered in which order, do not depend on the mode.
#[test]
fn ws4_tool_results_delivered_to_the_model_are_identical() {
    for session in [false, true] {
        let off = run(&config("tools", "off", session));
        let hybrid = run(&config("tools", "hybrid", session));
        assert!(off.tool_results.len() >= 40);
        assert_eq!(
            off.tool_results.len(),
            hybrid.tool_results.len(),
            "tool result counts differ"
        );
        for (index, (a, b)) in off
            .tool_results
            .iter()
            .zip(&hybrid.tool_results)
            .enumerate()
        {
            assert_eq!(a, b, "tool result {index} differs between off and hybrid");
        }
        // The same thing as the provider receives it.
        let outputs = |result: &RunResult| -> Vec<Value> {
            result.requests.last().unwrap().body["input"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|item| item["type"] == "function_call_output")
                .cloned()
                .collect()
        };
        assert_eq!(outputs(&off), outputs(&hybrid), "wire tool outputs differ");
        // Tool availability and order, at every request.
        for (index, (a, b)) in off.requests.iter().zip(&hybrid.requests).enumerate() {
            assert_eq!(
                a.body["tools"], b.body["tools"],
                "request {index}: tool definitions differ"
            );
        }
        assert_eq!(
            off.governor, hybrid.governor,
            "Token Governor counters differ"
        );
        assert_eq!(off.harness_messages, hybrid.harness_messages);
        println!(
            "WS_TOOLS session={session}: tool_results={} governor={} tools_per_request={}",
            off.tool_results.len(),
            off.governor,
            off.requests[0].body["tools"]
                .as_array()
                .map(Vec::len)
                .unwrap_or(0)
        );
    }
}

/// Q4.1, the other side of the line. A conversation that fills the context.
///
/// CURRENT BEHAVIOUR (documented, not forced green): with the default prune
/// profile, `off` and `hybrid` do NOT first diverge at compaction. `off`
/// calls `Agent::prune_context` every turn (turn.rs, the `else` arm of the
/// `active_context_vm / hybrid_context_vm` chain) and replaces old tool
/// results with a placeholder once the estimate passes 65% of the window
/// (`PruneSettings::cached_route`); `hybrid` skips that call. Compaction
/// starts at `window - reserveTokens`, which is later. So the first differing
/// request is the first one after an off-mode prune pass.
///
/// With pruning disabled (`DAVINCI_PRUNE_PROFILE=off`) the first differing
/// request is the first one after the earlier of the two modes' compactions.
#[test]
fn ws4_hybrid_first_diverges_at_compaction() {
    // 60k window: tools and instructions are about 17k tokens, compaction
    // starts at 60000 - 16384 = 43616, off-mode pruning at 39000.
    let filling = |mode: &str, prune_profile: Option<&str>| RunConfig {
        label: "fill".into(),
        mode: mode.into(),
        session: false,
        context_window: 60_000,
        script: Script::Filling(40),
        prune_profile: prune_profile.map(str::to_string),
        check_determinism: false,
    };
    let first_after = |result: &RunResult, counter: fn(&Captured) -> u64| {
        result
            .requests
            .iter()
            .position(|request| counter(request) > 0)
    };

    // (a) Pruning disabled: compaction is the only rewrite.
    let off = run(&filling("off", Some("off")));
    let hybrid = run(&filling("hybrid", Some("off")));
    summarize("fill off prune=off", &off);
    summarize("fill hybrid prune=off", &hybrid);
    assert_eq!(off.pruned_results, 0);
    let off_compacted = first_after(&off, |request| request.compactions);
    let hybrid_compacted = first_after(&hybrid, |request| request.compactions);
    let diverged = divergence_index(&off, &hybrid);
    println!(
        "WS_COMPACTION prune=off: first request after compaction off={off_compacted:?} hybrid={hybrid_compacted:?}; first differing request={diverged:?}\n  {}",
        first_divergence(&off, &hybrid).unwrap_or_else(|| "no divergence".into())
    );
    assert!(off_compacted.is_some(), "off never compacted");
    assert!(hybrid_compacted.is_some(), "hybrid never compacted");
    assert_eq!(
        diverged,
        off_compacted.into_iter().chain(hybrid_compacted).min(),
        "with pruning disabled the first differing request is the first one after a compaction"
    );

    // (b) Default settings: off prunes first.
    let off = run(&filling("off", None));
    let hybrid = run(&filling("hybrid", None));
    summarize("fill off prune=default", &off);
    summarize("fill hybrid prune=default", &hybrid);
    let off_pruned = first_after(&off, |request| request.pruned_results);
    let off_compacted = first_after(&off, |request| request.compactions);
    let hybrid_compacted = first_after(&hybrid, |request| request.compactions);
    let diverged = divergence_index(&off, &hybrid);
    println!(
        "WS_COMPACTION prune=default: first request after prune off={off_pruned:?}; after compaction off={off_compacted:?} hybrid={hybrid_compacted:?}; first differing request={diverged:?}\n  {}",
        first_divergence(&off, &hybrid).unwrap_or_else(|| "no divergence".into())
    );
    assert_eq!(hybrid.pruned_results, 0, "hybrid never prunes");
    // Current behaviour: the off-mode prune pass comes first and is where the
    // two modes part. If this starts failing because `diverged` equals the
    // compaction request instead, the documentation claim has become true.
    assert!(off_pruned.is_some(), "off never pruned");
    assert_eq!(
        diverged, off_pruned,
        "CURRENT BEHAVIOUR: off and hybrid first differ at the first request after an off-mode prune pass"
    );
    let earliest_compaction = off_compacted.into_iter().chain(hybrid_compacted).min();
    assert!(
        earliest_compaction.is_none_or(|compacted| off_pruned.unwrap() < compacted),
        "CURRENT BEHAVIOUR: the prune pass precedes every compaction (prune {off_pruned:?}, compaction {earliest_compaction:?})"
    );
}

/// Q4.3. Per-request harness cost of hybrid over off, model latency excluded.
/// `cargo test ... ws4_hybrid_per_turn_overhead -- --ignored --nocapture`
#[test]
#[ignore = "timing measurement; run with --ignored --nocapture"]
fn ws4_hybrid_per_turn_overhead() {
    const REPEATS: usize = 5;
    for session in [false, true] {
        let mut rows = Vec::new();
        for mode in ["off", "hybrid"] {
            let mut harness = Vec::new();
            let mut build = Vec::new();
            let mut requests = 0;
            for _ in 0..REPEATS {
                let result = run(&RunConfig {
                    script: Script::Timing,
                    ..config("timing", mode, session)
                });
                requests = result.requests.len();
                harness.push(result.harness_ms / requests as f64);
                build.push(result.request_build_ms / requests as f64);
            }
            harness.sort_by(|a, b| a.partial_cmp(b).unwrap());
            build.sort_by(|a, b| a.partial_cmp(b).unwrap());
            println!(
                "WS_TIMING session={session} mode={mode} requests={requests} repeats={REPEATS} harness_ms_per_request median={:.3} min={:.3} max={:.3}; request_build_ms_per_request median={:.3}",
                harness[REPEATS / 2],
                harness[0],
                harness[REPEATS - 1],
                build[REPEATS / 2],
            );
            rows.push(harness[REPEATS / 2]);
        }
        println!(
            "WS_TIMING session={session} hybrid_minus_off_ms_per_request={:.3}",
            rows[1] - rows[0]
        );
    }
}

// ---------------------------------------------------------------------------
// Part 2: append-only requests (mode off)
// ---------------------------------------------------------------------------

/// Q1.a. Each request must start with the previous request's cacheable
/// prefix. One assertion per scripted step kind, so a failure names the kind
/// of step whose output broke the prefix.
///
/// CURRENT BEHAVIOUR (documented, not forced green): every step kind extends
/// the previous request, inside one prompt's loop. The one violation is the
/// first request of the NEXT user prompt (`StepKind::UserPrompt`) when the
/// previous loop injected a completion reminder. During that loop the
/// reminder is sent in full as an overlay (`Agent::queue_completion_reminder`,
/// completion.rs: `completion_context` with `replaces_marker: true`, applied
/// by `completion_provider_history`). `run_loop_with` clears
/// `completion_context` when the loop ends (turn.rs), so from the next prompt
/// on the same input item is the 56-byte marker
/// `[DaVinci asked for a completion check before finishing.]`. An item that
/// was already sent changes, and everything after it leaves the cached
/// prefix. A single-prompt `--print` run (the benchmark) never reaches this:
/// the rewrite happens after its last request.
#[test]
fn ws1_requests_are_append_only_within_a_run() {
    const MARKER: &str = "[DaVinci asked for a completion check before finishing.]";
    for session in [false, true] {
        let result = run(&config("append", "off", session));
        assert!(result.requests.len() >= 40);
        let mut violations: std::collections::BTreeMap<StepKind, Vec<String>> = Default::default();
        let mut checked: std::collections::BTreeMap<StepKind, usize> = Default::default();
        for (index, pair) in result.requests.windows(2).enumerate() {
            let next = &pair[1];
            *checked.entry(next.after).or_default() += 1;
            if let Some(reason) = append_violation(&pair[0], next) {
                violations.entry(next.after).or_default().push(format!(
                    "request {} does not extend request {} (prompt {}): {reason}",
                    index + 1,
                    index,
                    next.prompt
                ));
            }
        }
        println!("WS_APPEND session={session}: transitions checked per step kind = {checked:?}");
        for (kind, found) in &violations {
            println!(
                "WS_APPEND session={session}: {kind:?} violations:\n  {}",
                found.join("\n  ")
            );
        }
        let of = |kind: StepKind| violations.get(&kind).cloned().unwrap_or_default();
        let exercised = |kind: StepKind| checked.get(&kind).copied().unwrap_or(0);
        macro_rules! extends {
            ($kind:expr) => {
                assert!(
                    exercised($kind) > 0,
                    "the script never exercised {:?}",
                    $kind
                );
                assert!(
                    of($kind).is_empty(),
                    "session={session}: request after a {:?} step is not a pure extension:\n{}",
                    $kind,
                    of($kind).join("\n")
                );
            };
        }
        extends!(StepKind::Ls);
        extends!(StepKind::PlainRead);
        extends!(StepKind::ParallelReads);
        extends!(StepKind::FailedRead);
        // The governor answers a repeated read with a short marker in the NEW
        // result; the earlier read's output is not touched.
        extends!(StepKind::RepeatedRead);
        // The governor compresses the NEW result; nothing earlier changes.
        extends!(StepKind::LargeLs);
        extends!(StepKind::Write);
        extends!(StepKind::Edit);
        // After the model's final text the harness appends a completion
        // reminder, then a verification reminder: both extend the request.
        extends!(StepKind::FinalText);
        extends!(StepKind::Followup);

        // The documented violation: the first request of the second prompt.
        assert_eq!(exercised(StepKind::UserPrompt), 1);
        let found = of(StepKind::UserPrompt);
        assert_eq!(
            found.len(),
            1,
            "CURRENT BEHAVIOUR changed: the request after a new user prompt now extends the previous one"
        );
        assert!(
            found[0].contains("$.input[")
                && found[0].contains(".content[0].text")
                && found[0].contains("Before finishing, review requirement coverage")
                && found[0].ends_with(MARKER),
            "the violating item is the completion reminder collapsing to its marker, got: {}",
            found[0]
        );
    }
}

/// Q1.b. Everything outside `input` is the same in every request of a run,
/// the cache key included; and it does not depend on having a session.
#[test]
fn ws1_cache_fields_are_stable_within_a_run() {
    let mut per_session = Vec::new();
    for session in [false, true] {
        let result = run(&config("fields", "off", session));
        let outside_input = |request: &Captured| {
            let mut body = request.body.clone();
            body.as_object_mut().unwrap().remove("input");
            body
        };
        let first = outside_input(&result.requests[0]);
        let mut keys: Vec<&String> = result.requests[0]
            .body
            .as_object()
            .unwrap()
            .keys()
            .collect();
        keys.sort();
        println!("WS_FIELDS session={session}: top-level keys = {keys:?}");
        println!(
            "WS_TOOLNAMES session={session}: {}",
            json!(first["tools"]
                .as_array()
                .unwrap()
                .iter()
                .map(|tool| tool["name"].clone())
                .collect::<Vec<_>>())
        );
        println!(
            "WS_FIELDS session={session}: {}",
            json!({
                "prompt_cache_key": first["prompt_cache_key"],
                "prompt_cache_options": first.get("prompt_cache_options"),
                "prompt_cache_retention": first.get("prompt_cache_retention"),
                "store": first["store"],
                "stream": first["stream"],
                "include": first["include"],
                "reasoning": first["reasoning"],
                "text": first["text"],
                "service_tier": first.get("service_tier"),
                "tool_choice": first["tool_choice"],
                "parallel_tool_calls": first["parallel_tool_calls"],
                "model": first["model"],
                "previous_response_id": first.get("previous_response_id"),
                "max_output_tokens": first.get("max_output_tokens"),
                "tools": first["tools"].as_array().map(Vec::len),
                "instructions_bytes": first["instructions"].as_str().map(str::len),
            })
        );
        for (index, request) in result.requests.iter().enumerate() {
            if let Some((path, x, y)) = first_diff(&first, &outside_input(request), "$") {
                panic!(
                    "session={session}: request {index} changed a non-input field at {path}: {} -> {}",
                    clip(&x),
                    clip(&y)
                );
            }
            assert!(
                request.body["prompt_cache_key"]
                    .as_str()
                    .is_some_and(|key| key.starts_with("ci-root-")),
                "request {index}: unexpected cache key {}",
                request.body["prompt_cache_key"]
            );
            for absent in [
                "previous_response_id",
                "prompt_cache_retention",
                "prompt_cache_options",
                "max_output_tokens",
                "conversation",
                "user",
                "metadata",
            ] {
                assert!(
                    request.body.get(absent).is_none(),
                    "request {index} carries {absent}"
                );
            }
            // No explicit cache breakpoint anywhere in the input.
            assert!(
                !request.body["input"]
                    .to_string()
                    .contains("prompt_cache_breakpoint"),
                "request {index} carries an explicit cache breakpoint"
            );
        }
        // One transport session id for the whole prompt's loop.
        for prompt in 0..2 {
            let ids: std::collections::BTreeSet<&String> = result
                .requests
                .iter()
                .filter(|request| request.prompt == prompt)
                .map(|request| &request.transport_session)
                .collect();
            assert_eq!(ids.len(), 1, "transport session id changed inside a prompt");
        }
        per_session.push((first, result));
    }
    // Session against --no-session: same non-input fields, same key.
    let (without, with) = (&per_session[0], &per_session[1]);
    assert_eq!(
        first_diff(&without.0, &with.0, "$"),
        None,
        "a session changes a non-input request field"
    );
    // Without a session each prompt's loop gets a fresh transport session id;
    // with one it is the durable session id. Neither reaches the body.
    let ids = |result: &RunResult| -> Vec<String> {
        (0..2)
            .map(|prompt| {
                result
                    .requests
                    .iter()
                    .find(|request| request.prompt == prompt)
                    .unwrap()
                    .transport_session
                    .clone()
            })
            .collect()
    };
    let (no_session_ids, session_ids) = (ids(&without.1), ids(&with.1));
    assert_ne!(
        no_session_ids[0], no_session_ids[1],
        "--no-session: a new transport session id per prompt"
    );
    assert_eq!(
        session_ids[0], session_ids[1],
        "session: one transport session id"
    );
    for result in [&without.1, &with.1] {
        for request in &result.requests {
            assert!(
                !request.wire.contains(&request.transport_session),
                "the transport session id reached the request body"
            );
        }
    }
}

/// Q1.c. The same request built twice is the same bytes, and two whole runs
/// of the same conversation produce the same bytes at every request.
#[test]
fn ws1_request_serialization_is_deterministic() {
    for session in [false, true] {
        let first = run(&RunConfig {
            check_determinism: true,
            ..config("determinism", "off", session)
        });
        let second = run(&config("determinism", "off", session));
        let divergence = first_divergence(&first, &second);
        assert!(
            divergence.is_none(),
            "session={session}: two runs of the same conversation differ:\n{}",
            divergence.unwrap_or_default()
        );
        // Object keys are emitted sorted (serde_json without preserve_order):
        // the byte order of the top-level fields is alphabetical.
        let text = first.requests[0].wire.clone();
        let positions: Vec<usize> = [
            "\"include\":",
            "\"input\":",
            "\"instructions\":",
            "\"model\":",
            "\"tools\":",
        ]
        .iter()
        .map(|key| text.find(key).unwrap_or_else(|| panic!("{key} missing")))
        .collect();
        println!("WS_ORDER session={session}: byte offsets include/input/instructions/model/tools = {positions:?} of {}", text.len());
        assert!(text.starts_with("{\"include\":"));
    }
}
