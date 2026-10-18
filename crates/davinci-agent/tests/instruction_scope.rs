//! Repository instructions over a session: nested `AGENTS.md` files reach the
//! model when it touches the directory they govern, edits to loaded files reach
//! the next request, and neither disturbs the cached prefix when nothing
//! changed.

use std::fs;
use std::path::Path;

use davinci_agent::{Agent, PermissionMode};
use davinci_ai::{content_text, AssistantMessage, ContentBlock, StopReason};

/// What one provider request carried.
#[derive(Debug, Clone)]
struct Request {
    system: String,
    messages: Vec<String>,
}

fn reply(turn: usize, call: Option<(&str, serde_json::Value)>) -> AssistantMessage {
    let (content, stop_reason) = match call {
        Some((name, arguments)) => (
            vec![ContentBlock::ToolCall {
                id: format!("call-{turn}"),
                name: name.into(),
                arguments,
            }],
            StopReason::ToolUse,
        ),
        None => (
            vec![ContentBlock::Text {
                text: "done".into(),
            }],
            StopReason::Stop,
        ),
    };
    AssistantMessage {
        extra: Default::default(),
        id: format!("assistant-{turn}"),
        role: "assistant".into(),
        content,
        model: "fixture".into(),
        usage: None,
        stop_reason: Some(stop_reason),
        error_message: None,
    }
}

/// Run one prompt whose model makes `calls` in order, then stops. `before`
/// runs inside the provider closure, before the request `turn` answers, which
/// is when a user edits a file mid-session. Returns each request's view.
fn run(
    agent: &mut Agent,
    calls: Vec<(&'static str, serde_json::Value)>,
    mut before: impl FnMut(usize),
) -> Vec<Request> {
    agent.prompt("go");
    let mut requests = Vec::new();
    let mut turn = 0;
    agent
        .run_loop(|current| {
            requests.push(Request {
                system: current.provider_system_prompt(),
                messages: current
                    .messages_for_provider()
                    .iter()
                    .map(|message| content_text(&message.content))
                    .collect(),
            });
            before(turn);
            let call = calls.get(turn).cloned();
            turn += 1;
            Ok(reply(turn, call))
        })
        .unwrap();
    requests
}

fn repository() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join(".git")).unwrap();
    fs::write(root.join("AGENTS.md"), "root rules").unwrap();
    for sub in ["sub/deep", "other"] {
        fs::create_dir_all(root.join(sub)).unwrap();
    }
    fs::write(root.join("sub/AGENTS.md"), "sub rules").unwrap();
    fs::write(root.join("sub/deep/AGENTS.md"), "deep rules").unwrap();
    fs::write(root.join("other/AGENTS.md"), "other rules").unwrap();
    fs::write(root.join("sub/x.rs"), "fn x() {}").unwrap();
    fs::write(root.join("sub/deep/y.rs"), "fn y() {}").unwrap();
    fs::write(root.join("other/z.rs"), "fn z() {}").unwrap();
    dir
}

fn agent_in(root: &Path, enabled: bool) -> Agent {
    let mut agent = Agent::new("base prompt");
    agent.cwd = root.to_path_buf();
    agent.set_permission_mode(PermissionMode::Auto);
    agent.load_instruction_files(enabled);
    agent
}

fn nested(request: &Request) -> Vec<&String> {
    request
        .messages
        .iter()
        .filter(|text| text.contains("<nested_instructions>"))
        .collect()
}

#[test]
fn a_read_delivers_the_nested_file_scoped_to_its_directory() {
    let repo = repository();
    let mut agent = agent_in(repo.path(), true);
    let requests = run(
        &mut agent,
        vec![("read", serde_json::json!({"path": "sub/x.rs"}))],
        |_| {},
    );
    assert_eq!(requests.len(), 2);
    assert!(nested(&requests[0]).is_empty());

    let delivered = nested(&requests[1]);
    assert_eq!(delivered.len(), 1, "{:#?}", requests[1].messages);
    let text = delivered[0];
    assert!(text.contains("sub rules"), "{text}");
    assert!(text.contains(r#"scope="sub/""#), "{text}");
    assert!(text.contains("only to files under the directory"), "{text}");
    // Root's file is in the system prompt already; others are not governing.
    assert!(!text.contains("root rules") && !text.contains("deep rules"));
    assert!(!requests[1].messages.join("\n").contains("other rules"));
    assert!(!requests[1].system.contains("sub rules"));
}

#[test]
fn active_context_vm_keeps_the_native_tool_pair_on_the_delivering_request() {
    use davinci_agent::runtime::context_vm::ContextVmMode;
    use davinci_agent::{AgentId, RunId, RuntimeBus, RuntimeHandle};
    use davinci_ai::{ChatMessage, MessageContent};

    let repo = repository();
    let mut agent = agent_in(repo.path(), true);
    agent.set_runtime(RuntimeHandle::new(
        RunId::new(),
        AgentId::new(),
        RuntimeBus::new(),
    ));
    agent.set_context_vm_mode(ContextVmMode::Active);
    agent.prompt("go");
    let mut requests: Vec<(String, Vec<ChatMessage>)> = Vec::new();
    let mut turn = 0;
    agent
        .run_loop(|current| {
            requests.push((
                current.provider_system_prompt(),
                current.messages_for_provider(),
            ));
            let call = (turn == 0).then(|| ("read", serde_json::json!({"path": "sub/x.rs"})));
            turn += 1;
            Ok(reply(turn, call))
        })
        .unwrap();
    assert_eq!(requests.len(), 2);

    let (system, messages) = &requests[1];
    let paired = messages.windows(2).any(|pair| {
        pair[0]
            .content
            .iter()
            .any(|block| matches!(block, MessageContent::ToolCall { id, .. } if id == "call-1"))
            && pair[1].role == "toolResult"
            && pair[1].tool_call_id.as_deref() == Some("call-1")
    });
    let texts: Vec<String> = messages.iter().map(|m| content_text(&m.content)).collect();
    assert!(paired, "native call/result pair lost: {texts:#?}");
    let delivered = texts
        .iter()
        .filter(|text| text.contains("<nested_instructions>"))
        .count();
    assert_eq!(delivered, 1, "{texts:#?}");
    assert!(texts.iter().any(|text| text.contains("sub rules")));
    assert!(!system.contains("sub rules"), "the cached prefix changed");
}

#[test]
fn a_delivery_does_not_cost_the_active_ledger_an_extra_rebuild() {
    use davinci_agent::runtime::context_vm::ContextVmMode;
    use davinci_agent::{AgentId, RunId, RuntimeBus, RuntimeHandle};

    // Two reads then a final answer; `enabled` decides whether the first read
    // delivers sub/AGENTS.md (the second finds it delivered already).
    let rebuilds = |enabled: bool| {
        let repo = repository();
        let mut agent = agent_in(repo.path(), enabled);
        agent.set_runtime(RuntimeHandle::new(
            RunId::new(),
            AgentId::new(),
            RuntimeBus::new(),
        ));
        agent.set_context_vm_mode(ContextVmMode::Active);
        let sessions = tempfile::tempdir().unwrap();
        agent.session =
            Some(davinci_session::JsonlSession::create(sessions.path(), "ledger", None).unwrap());
        run(
            &mut agent,
            vec![
                ("read", serde_json::json!({"path": "sub/x.rs"})),
                ("read", serde_json::json!({"path": "sub/x.rs"})),
            ],
            |_| {},
        );
        agent
            .runtime
            .as_ref()
            .unwrap()
            .context_vm
            .metrics()
            .rebuild_attempts
    };
    let (with, without) = (rebuilds(true), rebuilds(false));
    assert_eq!(with, without);
}

#[test]
fn a_nested_delivery_never_fails_a_request_that_fits_without_it() {
    use davinci_agent::runtime::context_vm::ContextVmMode;
    use davinci_agent::{AgentId, RunId, RuntimeBus, RuntimeHandle};
    use davinci_ai::{ChatMessage, MessageContent};

    // The request right after a tool batch with a large result, with or
    // without a small delivered nested block, at a given window.
    let compiles = |window: u64, nested: bool| {
        let mut agent = Agent::new("base prompt");
        agent.set_runtime(RuntimeHandle::new(
            RunId::new(),
            AgentId::new(),
            RuntimeBus::new(),
        ));
        agent.set_context_vm_mode(ContextVmMode::Active);
        agent.context_window = window;
        agent.messages.push(ChatMessage::text("user", "go"));
        let mut call = ChatMessage::text("assistant", "");
        call.content = vec![MessageContent::ToolCall {
            id: "call-1".into(),
            name: "read".into(),
            arguments: serde_json::json!({"path": "sub/x.rs"}),
        }];
        agent.messages.push(call);
        let mut result = ChatMessage::text(
            "toolResult",
            "fn x() {}
"
            .repeat(4_000),
        );
        result.tool_call_id = Some("call-1".into());
        agent.messages.push(result);
        if nested {
            agent.record_custom_message(&serde_json::json!({
                "customType": "davinci.nested_instructions",
                "content": "rule ".repeat(40),
                "display": false,
                "details": {"files": []},
            }));
        }
        match agent.prepared_context_image() {
            Ok(_) => true,
            Err(error) => {
                assert!(
                    error.contains("exceeds the compilation token budget"),
                    "{error}"
                );
                false
            }
        }
    };
    let smallest = |nested: bool| {
        let (mut low, mut high) = (1_000u64, 400_000u64);
        assert!(compiles(high, nested) && !compiles(low, nested));
        while low + 1 < high {
            let middle = (low + high) / 2;
            if compiles(middle, nested) {
                high = middle;
            } else {
                low = middle;
            }
        }
        high
    };
    // The nested block is reserved with the native pair only while that fits;
    // it must never need a larger window than the request without it.
    let (without, with) = (smallest(false), smallest(true));
    assert!(
        with <= without,
        "the nested block raised the smallest working window from {without} to {with}"
    );
}

#[test]
fn delivery_leaves_the_cached_prefix_untouched() {
    let repo = repository();
    let mut agent = agent_in(repo.path(), true);
    let requests = run(
        &mut agent,
        vec![("read", serde_json::json!({"path": "sub/x.rs"}))],
        |_| {},
    );
    // Same system prompt, and request 0's messages are a strict prefix of
    // request 1's: the new instructions were appended, nothing was rewritten.
    assert_eq!(requests[0].system, requests[1].system);
    assert_eq!(
        requests[0].messages[..],
        requests[1].messages[..requests[0].messages.len()]
    );
    // The instructions arrive as the last message, after the tool result.
    assert!(requests[1]
        .messages
        .last()
        .unwrap()
        .contains("<nested_instructions>"));
}

#[test]
fn a_write_into_a_new_file_and_an_edit_load_their_governing_file() {
    let repo = repository();
    let mut agent = agent_in(repo.path(), true);
    let requests = run(
        &mut agent,
        vec![
            (
                "write",
                serde_json::json!({"path": "sub/deep/new/created.rs", "content": "fn c() {}"}),
            ),
            (
                "edit",
                serde_json::json!({
                    "path": "other/z.rs",
                    "edits": [{"oldText": "fn z() {}", "newText": "fn z2() {}"}]
                }),
            ),
        ],
        |_| {},
    );
    let after_write = nested(&requests[1]);
    assert_eq!(after_write.len(), 1, "{:#?}", requests[1].messages);
    // The new file's directory does not exist yet: sub/ and sub/deep/ govern it.
    assert!(after_write[0].contains("sub rules") && after_write[0].contains("deep rules"));
    assert!(!after_write[0].contains("other rules"));
    assert!(after_write[0].contains(r#"scope="sub/deep/""#));

    let after_edit = nested(&requests[2]);
    assert_eq!(after_edit.len(), 2, "{:#?}", requests[2].messages);
    assert!(after_edit[1].contains("other rules"));
    assert!(!after_edit[1].contains("sub rules"));
}

#[test]
fn a_file_is_delivered_once_until_it_changes() {
    let repo = repository();
    let mut agent = agent_in(repo.path(), true);
    let read = |path: &str| ("read", serde_json::json!({"path": path}));
    let requests = run(
        &mut agent,
        vec![read("sub/x.rs"), read("sub/x.rs"), read("sub/deep/y.rs")],
        |_| {},
    );
    assert_eq!(nested(&requests[1]).len(), 1);
    assert_eq!(nested(&requests[2]).len(), 1, "a repeat touch adds nothing");
    // A deeper file is new: only it is added, not sub/ again.
    let last = nested(&requests[3]);
    assert_eq!(last.len(), 2);
    assert!(last[1].contains("deep rules") && !last[1].contains("sub rules"));

    // An edit to the nested file is delivered again on the next touch.
    fs::write(repo.path().join("sub/AGENTS.md"), "sub rules, revised").unwrap();
    let again = run(
        &mut agent,
        vec![("read", serde_json::json!({"path": "sub/x.rs"}))],
        |_| {},
    );
    assert!(nested(&again[1])
        .last()
        .unwrap()
        .contains("sub rules, revised"));
}

#[test]
fn a_failed_tool_call_and_a_path_outside_the_checkout_load_nothing() {
    let repo = repository();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("AGENTS.md"), "outside rules").unwrap();
    fs::write(outside.path().join("f.rs"), "").unwrap();
    let mut agent = agent_in(repo.path(), true);
    let requests = run(
        &mut agent,
        vec![
            ("read", serde_json::json!({"path": "sub/missing.rs"})),
            (
                "read",
                serde_json::json!({"path": outside.path().join("f.rs")}),
            ),
        ],
        |_| {},
    );
    assert!(requests.iter().all(|request| nested(request).is_empty()));
}

#[test]
fn the_no_context_files_flag_loads_nothing_now_or_later() {
    let repo = repository();
    let mut agent = agent_in(repo.path(), false);
    assert!(agent.context_files.is_empty());
    let requests = run(
        &mut agent,
        vec![("read", serde_json::json!({"path": "sub/x.rs"}))],
        |_| {},
    );
    for request in &requests {
        assert!(nested(request).is_empty());
        assert!(!request.system.contains("rules"));
    }
}

#[test]
fn an_edited_instruction_file_reaches_the_next_request() {
    let repo = repository();
    let mut agent = agent_in(repo.path(), true);
    let agents = repo.path().join("AGENTS.md");
    let requests = run(
        &mut agent,
        vec![("read", serde_json::json!({"path": "other/z.rs"}))],
        |turn| {
            if turn == 0 {
                fs::write(&agents, "root rules, now with a new rule about tests").unwrap();
            }
        },
    );
    assert!(requests[0].system.contains("root rules"));
    assert!(!requests[0].system.contains("new rule about tests"));
    assert!(
        requests[1].system.contains("new rule about tests"),
        "{}",
        requests[1].system
    );
    assert!(!requests[1].system.contains("root rules\n"));
}

#[test]
fn a_removed_instruction_file_leaves_the_prompt() {
    let repo = repository();
    let mut agent = agent_in(repo.path(), true);
    let agents = repo.path().join("AGENTS.md");
    let requests = run(
        &mut agent,
        vec![("read", serde_json::json!({"path": "other/z.rs"}))],
        |turn| {
            if turn == 0 {
                fs::remove_file(&agents).unwrap();
            }
        },
    );
    assert!(requests[0].system.contains("root rules"));
    assert!(!requests[1].system.contains("root rules"));
    assert!(!requests[1].system.contains("<project_context>"));
}

#[test]
fn without_an_edit_the_system_prompt_is_byte_identical() {
    let repo = repository();
    let mut agent = agent_in(repo.path(), true);
    let requests = run(
        &mut agent,
        vec![
            ("read", serde_json::json!({"path": "other/z.rs"})),
            ("read", serde_json::json!({"path": "sub/x.rs"})),
        ],
        |_| {},
    );
    assert!(requests.len() == 3);
    assert!(requests
        .iter()
        .all(|request| request.system == requests[0].system));
    assert!(requests[0].system.contains("root rules"));
}

#[test]
fn vendored_and_build_directories_never_deliver_instructions() {
    let repo = repository();
    for dir in [
        "node_modules/pkg",
        "sub/vendor/lib",
        "target/debug",
        "sub/build",
    ] {
        fs::create_dir_all(repo.path().join(dir)).unwrap();
        fs::write(repo.path().join(dir).join("AGENTS.md"), "vendored rules").unwrap();
        fs::write(repo.path().join(dir).join("index.js"), "").unwrap();
    }
    let mut agent = agent_in(repo.path(), true);
    let read = |path: &str| ("read", serde_json::json!({"path": path}));
    let requests = run(
        &mut agent,
        vec![
            read("node_modules/pkg/index.js"),
            read("sub/vendor/lib/index.js"),
            read("target/debug/index.js"),
            read("sub/build/index.js"),
        ],
        |_| {},
    );
    for request in &requests {
        assert!(
            !request.messages.join("\n").contains("vendored rules"),
            "{:#?}",
            request.messages
        );
    }
    // `sub/` itself still governs what sits inside its vendored tree.
    assert_eq!(nested(requests.last().unwrap()).len(), 1);
    assert!(nested(requests.last().unwrap())[0].contains("sub rules"));
}

#[test]
fn excluded_directory_names_match_case_insensitively() {
    // Windows and macOS filesystems are case-insensitive: `Node_Modules` is
    // the same directory as `node_modules` there.
    let repo = repository();
    for dir in ["Node_Modules/pkg", "sub/VENDOR/lib"] {
        fs::create_dir_all(repo.path().join(dir)).unwrap();
        fs::write(repo.path().join(dir).join("AGENTS.md"), "vendored rules").unwrap();
        fs::write(repo.path().join(dir).join("index.js"), "").unwrap();
    }
    let mut agent = agent_in(repo.path(), true);
    let read = |path: &str| ("read", serde_json::json!({"path": path}));
    let requests = run(
        &mut agent,
        vec![
            read("Node_Modules/pkg/index.js"),
            read("sub/VENDOR/lib/index.js"),
        ],
        |_| {},
    );
    for request in &requests {
        assert!(
            !request.messages.join("\n").contains("vendored rules"),
            "{:#?}",
            request.messages
        );
    }
}

#[test]
fn more_dependency_trees_never_deliver_instructions() {
    let repo = repository();
    let dirs = [".venv/lib", "venv/lib", "Pods/lib", "bower_components/lib"];
    for dir in dirs {
        fs::create_dir_all(repo.path().join(dir)).unwrap();
        fs::write(repo.path().join(dir).join("AGENTS.md"), "vendored rules").unwrap();
        fs::write(repo.path().join(dir).join("index.js"), "").unwrap();
    }
    let mut agent = agent_in(repo.path(), true);
    let calls = dirs
        .iter()
        .map(|dir| {
            (
                "read",
                serde_json::json!({"path": format!("{dir}/index.js")}),
            )
        })
        .collect();
    let requests = run(&mut agent, calls, |_| {});
    for request in &requests {
        assert!(
            !request
                .messages
                .join(
                    "
"
                )
                .contains("vendored rules"),
            "{:#?}",
            request.messages
        );
    }
}

#[test]
fn a_nested_repository_is_not_governed_by_or_delivered_from() {
    let repo = repository();
    let inner = repo.path().join("sub/inner");
    fs::create_dir_all(inner.join("src")).unwrap();
    // A submodule's `.git` is a file; a nested clone's is a directory.
    fs::write(inner.join(".git"), "gitdir: ../../.git/modules/inner").unwrap();
    fs::write(inner.join("AGENTS.md"), "inner repo rules").unwrap();
    fs::write(inner.join("src/AGENTS.md"), "inner src rules").unwrap();
    fs::write(inner.join("src/lib.rs"), "").unwrap();
    let mut agent = agent_in(repo.path(), true);
    let requests = run(
        &mut agent,
        vec![("read", serde_json::json!({"path": "sub/inner/src/lib.rs"}))],
        |_| {},
    );
    for request in &requests {
        assert!(
            !request
                .messages
                .join(
                    "
"
                )
                .contains("inner"),
            "{:#?}",
            request.messages
        );
    }
}

#[test]
fn an_instruction_file_created_mid_session_reaches_the_next_request() {
    let repo = repository();
    fs::remove_file(repo.path().join("AGENTS.md")).unwrap();
    fs::remove_file(repo.path().join("sub/AGENTS.md")).unwrap();
    let cwd = repo.path().join("sub");
    let mut agent = agent_in(&cwd, true);
    assert!(agent.context_files.iter().all(|f| !f.body.contains("late")));
    let root_file = repo.path().join("AGENTS.md");
    let sub_file = cwd.join("AGENTS.md");
    let requests = run(
        &mut agent,
        vec![("read", serde_json::json!({"path": "x.rs"}))],
        |turn| {
            if turn == 0 {
                // Created in the reverse of chain order on purpose.
                fs::write(&sub_file, "late sub rules").unwrap();
                fs::write(&root_file, "late root rules").unwrap();
            }
        },
    );
    assert!(!requests[0].system.contains("late"));
    let system = &requests[1].system;
    let (root, sub) = (
        system.find("late root rules"),
        system.find("late sub rules"),
    );
    assert!(root.is_some() && sub.is_some(), "{system}");
    assert!(root < sub, "outermost file reads first: {system}");
}

#[test]
fn a_symlink_leaving_the_checkout_is_not_followed() {
    let repo = repository();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("AGENTS.md"), "outside rules").unwrap();
    fs::write(outside.path().join("f.rs"), "").unwrap();
    let link_dir = repo.path().join("escape");
    let link_file = repo.path().join("other/AGENTS.md");
    fs::remove_file(&link_file).unwrap();
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(outside.path(), &link_dir).is_ok()
        && std::os::unix::fs::symlink(outside.path().join("AGENTS.md"), &link_file).is_ok();
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_dir(outside.path(), &link_dir).is_ok()
        && std::os::windows::fs::symlink_file(outside.path().join("AGENTS.md"), &link_file).is_ok();
    if !linked {
        // Symlinks need privilege on some hosts (Windows without developer mode).
        eprintln!("SKIPPED: symlink creation failed, the escape case did not run");
        return;
    }
    let mut agent = agent_in(repo.path(), true);
    let read = |path: &str| ("read", serde_json::json!({"path": path}));
    let requests = run(
        &mut agent,
        vec![read("escape/f.rs"), read("other/z.rs")],
        |_| {},
    );
    for request in &requests {
        assert!(
            !request.messages.join("\n").contains("outside rules"),
            "{:#?}",
            request.messages
        );
    }
}
