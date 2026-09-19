//! Integration tests for P8 Deterministic Hook / Policy Engine.
//! Covers all 14 completion gates:
//! 1. Schema & structured rule matching (event, tool, path glob)
//! 2. Legacy hooks backward compatibility (preTool, postTool, stop)
//! 3. Failure policies (block, warn, ignore)
//! 4. Untrusted project hooks rejection
//! 5. Post-load content identity SHA-256 tamper invalidation
//! 6. BeforeWrite decision event blocking preventing mutation
//! 7. AfterWrite observer failure creating unmet completion requirement
//! 8. BeforeProcessStart decision event blocking preventing spawn
//! 9. Host-owned recursion depth guard (max depth 3)
//! 10. Bounded streams (64 KiB) and process timeout termination
//! 11. Diagnostics command `/hook-status` reporting

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use davinci_agent::runtime::bus::RuntimeBus;
use davinci_agent::runtime::events::{RuntimeEvent, RuntimeEventEnvelope};
use davinci_agent::tools::{execute_tool_with, ToolContext};
use davinci_agent::{AgentId, RunId};
use davinci_coding_agent::hooks::{
    self, HookDepthGuard, HookFailurePolicy, HookPolicyConfig, HookPolicyRule, HooksFile,
    GLOBAL_HOOK_TELEMETRY,
};
use davinci_coding_agent::native_extensions::NativeExtensionHost;
use davinci_coding_agent::runtime_host::HooksRuntimeSubscriber;
use serde_json::json;
use tempfile::TempDir;

struct TestEnv {
    _temp_dir: TempDir,
    project_dir: PathBuf,
    agent_dir: PathBuf,
}

impl TestEnv {
    fn new() -> Self {
        let temp_dir = TempDir::new().unwrap();
        let project_dir = temp_dir.path().join("project");
        let agent_dir = temp_dir.path().join("agent");
        fs::create_dir_all(&project_dir).unwrap();
        fs::create_dir_all(&agent_dir).unwrap();
        Self {
            _temp_dir: temp_dir,
            project_dir,
            agent_dir,
        }
    }

    fn write_project_hooks(&self, json_content: &str) -> PathBuf {
        let pi_dir = self.project_dir.join(".pi");
        fs::create_dir_all(&pi_dir).unwrap();
        let hooks_path = pi_dir.join("hooks.json");
        fs::write(&hooks_path, json_content).unwrap();
        hooks_path
    }
}

fn publish_decision(bus: &RuntimeBus, payload: RuntimeEvent) -> Result<(), String> {
    let env = RuntimeEventEnvelope::new(1, RunId::new(), None, Some(AgentId::new()), None, payload);
    bus.emit_decision(env)
}

fn publish_observe(bus: &RuntimeBus, payload: RuntimeEvent) {
    let env = RuntimeEventEnvelope::new(1, RunId::new(), None, Some(AgentId::new()), None, payload);
    bus.emit_observe(env);
}

#[test]
fn test_legacy_hooks_backward_compatibility() {
    let env = TestEnv::new();
    let hooks_json = json!({
        "preTool": [
            ["node", "-e", "console.error('legacy preTool blocked'); process.exit(1)"]
        ],
        "postTool": [
            ["node", "-e", "process.exit(0)"]
        ]
    });
    env.write_project_hooks(&hooks_json.to_string());

    let hooks = hooks::load(&env.agent_dir, &env.project_dir, true);
    assert!(hooks.project_trusted);
    assert_eq!(hooks.pre_tool.len(), 1);
    assert_eq!(hooks.post_tool.len(), 1);

    let subscriber = HooksRuntimeSubscriber::new_with_config(
        hooks,
        HookPolicyConfig::default(),
        env.project_dir.clone(),
        env.agent_dir.clone(),
    );

    let bus = RuntimeBus::new();
    bus.subscribe(Arc::new(subscriber));

    // PreToolUse should be blocked by legacy preTool
    let pre_event = RuntimeEvent::PreToolUse {
        tool: "write".to_string(),
        args: json!({"path": "file.txt", "content": "hello"}),
        call_id: "call-1".to_string(),
    };
    let decision = publish_decision(&bus, pre_event);
    match decision {
        Err(reason) => {
            assert!(
                reason.contains("legacy preTool blocked"),
                "Expected reason to contain failure output, got: {reason}"
            );
        }
        Ok(()) => panic!("Expected legacy preTool failure to deny PreToolUse"),
    }

    // PostToolUse should execute without error
    let post_event = RuntimeEvent::PostToolUse {
        tool: "write".to_string(),
        call_id: "call-1".to_string(),
        is_error: false,
    };
    publish_observe(&bus, post_event);
}

#[test]
fn test_hook_policy_rules_matching() {
    let rule = HookPolicyRule {
        event: "beforeWrite".to_string(),
        tool: Some("write".to_string()),
        path_pattern: Some("*.ts".to_string()),
        action: vec!["node".into(), "-e".into(), "process.exit(1)".into()],
        timeout_ms: Some(5000),
        on_failure: HookFailurePolicy::Block,
    };

    // Correct event, tool, and path match
    assert!(rule.matches("beforeWrite", "write", Some(Path::new("src/index.ts"))));
    assert!(rule.matches("BeforeWrite", "write", Some(Path::new("index.ts"))));
    assert!(rule.matches("before_write", "write", Some(Path::new("C:/repo/app.ts"))));

    // Mismatched path
    assert!(!rule.matches("beforeWrite", "write", Some(Path::new("src/index.rs"))));

    // Mismatched tool
    assert!(!rule.matches("beforeWrite", "edit", Some(Path::new("src/index.ts"))));

    // Mismatched event
    assert!(!rule.matches("afterWrite", "write", Some(Path::new("src/index.ts"))));

    // Rule with no path or tool filter matches any tool/path for that event
    let broad_rule = HookPolicyRule {
        event: "beforeProcessStart".to_string(),
        tool: None,
        path_pattern: None,
        action: vec!["node".into(), "-e".into(), "process.exit(0)".into()],
        timeout_ms: None,
        on_failure: HookFailurePolicy::Block,
    };
    assert!(broad_rule.matches("beforeProcessStart", "process_start", None));
    assert!(broad_rule.matches(
        "before_process_start",
        "anything",
        Some(Path::new("any/path"))
    ));
}

#[test]
fn test_hook_failure_policies() {
    let env = TestEnv::new();

    // 1. Block policy
    let block_rule = HookPolicyRule {
        event: "beforeWrite".to_string(),
        tool: None,
        path_pattern: None,
        action: vec![
            "node".into(),
            "-e".into(),
            "console.error('block policy triggered'); process.exit(1)".into(),
        ],
        timeout_ms: Some(5000),
        on_failure: HookFailurePolicy::Block,
    };
    let hooks_block = HooksFile {
        project_trusted: true,
        rules: vec![block_rule],
        ..Default::default()
    };

    let subscriber_block = HooksRuntimeSubscriber::new_with_config(
        hooks_block,
        HookPolicyConfig::default(),
        env.project_dir.clone(),
        env.agent_dir.clone(),
    );
    let bus_block = RuntimeBus::new();
    bus_block.subscribe(Arc::new(subscriber_block));

    let decision = publish_decision(
        &bus_block,
        RuntimeEvent::BeforeWrite {
            path: env.project_dir.join("test.txt"),
            bytes: 10,
        },
    );
    assert!(decision.is_err());

    // 2. Warn policy
    let warn_rule = HookPolicyRule {
        event: "beforeWrite".to_string(),
        tool: None,
        path_pattern: None,
        action: vec![
            "node".into(),
            "-e".into(),
            "console.error('warn policy triggered'); process.exit(1)".into(),
        ],
        timeout_ms: Some(5000),
        on_failure: HookFailurePolicy::Warn,
    };
    let hooks_warn = HooksFile {
        project_trusted: true,
        rules: vec![warn_rule],
        ..Default::default()
    };

    let initial_warned = GLOBAL_HOOK_TELEMETRY.warned.load(Ordering::Relaxed);
    let subscriber_warn = HooksRuntimeSubscriber::new_with_config(
        hooks_warn,
        HookPolicyConfig::default(),
        env.project_dir.clone(),
        env.agent_dir.clone(),
    );
    let bus_warn = RuntimeBus::new();
    bus_warn.subscribe(Arc::new(subscriber_warn));

    let decision = publish_decision(
        &bus_warn,
        RuntimeEvent::BeforeWrite {
            path: env.project_dir.join("test.txt"),
            bytes: 10,
        },
    );
    assert!(decision.is_ok());
    assert_eq!(
        GLOBAL_HOOK_TELEMETRY.warned.load(Ordering::Relaxed),
        initial_warned + 1
    );

    // 3. Ignore policy
    let ignore_rule = HookPolicyRule {
        event: "beforeWrite".to_string(),
        tool: None,
        path_pattern: None,
        action: vec!["node".into(), "-e".into(), "process.exit(1)".into()],
        timeout_ms: Some(5000),
        on_failure: HookFailurePolicy::Ignore,
    };
    let hooks_ignore = HooksFile {
        project_trusted: true,
        rules: vec![ignore_rule],
        ..Default::default()
    };

    let initial_ignored = GLOBAL_HOOK_TELEMETRY.ignored.load(Ordering::Relaxed);
    let subscriber_ignore = HooksRuntimeSubscriber::new_with_config(
        hooks_ignore,
        HookPolicyConfig::default(),
        env.project_dir.clone(),
        env.agent_dir.clone(),
    );
    let bus_ignore = RuntimeBus::new();
    bus_ignore.subscribe(Arc::new(subscriber_ignore));

    let decision = publish_decision(
        &bus_ignore,
        RuntimeEvent::BeforeWrite {
            path: env.project_dir.join("test.txt"),
            bytes: 10,
        },
    );
    assert!(decision.is_ok());
    assert_eq!(
        GLOBAL_HOOK_TELEMETRY.ignored.load(Ordering::Relaxed),
        initial_ignored + 1
    );
}

#[test]
fn test_untrusted_project_hooks_rejected() {
    let env = TestEnv::new();
    let hooks_json = json!({
        "rules": [
            {
                "event": "beforeWrite",
                "action": ["node", "-e", "process.exit(1)"],
                "onFailure": "block"
            }
        ]
    });
    env.write_project_hooks(&hooks_json.to_string());

    // Load with project_trusted = false
    let hooks = hooks::load(&env.agent_dir, &env.project_dir, false);
    assert!(!hooks.project_trusted);
    assert_eq!(
        hooks.rules.len(),
        0,
        "Untrusted project rules must not be loaded"
    );

    let subscriber = HooksRuntimeSubscriber::new_with_config(
        hooks,
        HookPolicyConfig::default(),
        env.project_dir.clone(),
        env.agent_dir.clone(),
    );
    let bus = RuntimeBus::new();
    bus.subscribe(Arc::new(subscriber));

    // Decision event is NOT denied because untrusted hooks are ignored
    let decision = publish_decision(
        &bus,
        RuntimeEvent::BeforeWrite {
            path: env.project_dir.join("test.txt"),
            bytes: 10,
        },
    );
    assert!(decision.is_ok());
}

#[test]
fn test_post_load_file_modification_invalidation() {
    let env = TestEnv::new();
    let hooks_json = json!({
        "rules": [
            {
                "event": "beforeWrite",
                "action": ["node", "-e", "process.exit(0)"],
                "onFailure": "block"
            }
        ]
    });
    let hooks_path = env.write_project_hooks(&hooks_json.to_string());

    // Load hooks legitimately
    let hooks = hooks::load(&env.agent_dir, &env.project_dir, true);
    assert!(hooks.project_trusted);
    assert!(hooks.content_hash.is_some());

    let subscriber = HooksRuntimeSubscriber::new_with_config(
        hooks,
        HookPolicyConfig::default(),
        env.project_dir.clone(),
        env.agent_dir.clone(),
    );
    let bus = RuntimeBus::new();
    bus.subscribe(Arc::new(subscriber));

    // First call before tamper passes
    let dec1 = publish_decision(
        &bus,
        RuntimeEvent::BeforeWrite {
            path: env.project_dir.join("test.txt"),
            bytes: 10,
        },
    );
    assert!(dec1.is_ok());

    // Tamper with hooks.json on disk
    fs::write(&hooks_path, r#"{"tampered": true}"#).unwrap();

    // Next call detects SHA-256 hash mismatch and fails closed
    let dec2 = publish_decision(
        &bus,
        RuntimeEvent::BeforeWrite {
            path: env.project_dir.join("test.txt"),
            bytes: 10,
        },
    );
    match dec2 {
        Err(reason) => {
            assert!(
                reason.contains("modified on disk"),
                "Expected tamper detection error, got: {reason}"
            );
        }
        Ok(()) => panic!("Expected post-load modification to fail closed with Deny"),
    }
}

#[test]
fn test_before_write_blocking_prevents_file_mutation() {
    let env = TestEnv::new();
    let real_file = env.project_dir.join("test_file.txt");
    fs::write(&real_file, "initial content").unwrap();

    // Hook blocks write to test_file.txt
    let block_rule = HookPolicyRule {
        event: "beforeWrite".to_string(),
        tool: None,
        path_pattern: Some("test_file.txt".to_string()),
        action: vec![
            "node".into(),
            "-e".into(),
            "console.error('mutation prohibited'); process.exit(1)".into(),
        ],
        timeout_ms: Some(5000),
        on_failure: HookFailurePolicy::Block,
    };
    let hooks = HooksFile {
        project_trusted: true,
        rules: vec![block_rule],
        ..Default::default()
    };

    let bus = RuntimeBus::new();
    let subscriber = HooksRuntimeSubscriber::new_with_config(
        hooks,
        HookPolicyConfig::default(),
        env.project_dir.clone(),
        env.agent_dir.clone(),
    );
    bus.subscribe(Arc::new(subscriber));

    let run_id = RunId::new();
    let agent_id = AgentId::new();
    let runtime_handle = davinci_agent::RuntimeHandle::new(run_id, agent_id, bus.clone());

    let context = ToolContext {
        runtime: Some(runtime_handle),
        ..Default::default()
    };

    // Writing to test_file.txt should be blocked by BeforeWrite hook
    let write_res = execute_tool_with(
        &env.project_dir,
        "write",
        &json!({"path": "test_file.txt", "content": "mutated content"}),
        &context,
    );
    assert!(
        write_res.is_err() || write_res.as_ref().is_ok_and(|r| r.is_error),
        "Expected write to be blocked by BeforeWrite hook"
    );
    assert_eq!(
        fs::read_to_string(&real_file).unwrap(),
        "initial content",
        "File content must NOT be changed on disk when BeforeWrite blocks"
    );

    // Writing to allowed.txt should succeed
    let allowed_file = env.project_dir.join("allowed.txt");
    let ok_res = execute_tool_with(
        &env.project_dir,
        "write",
        &json!({"path": "allowed.txt", "content": "safe content"}),
        &context,
    );
    assert!(ok_res.is_ok() && !ok_res.as_ref().unwrap().is_error);
    assert!(allowed_file.exists());
    assert_eq!(fs::read_to_string(&allowed_file).unwrap(), "safe content");
}

#[test]
fn test_after_write_observer_failure_records_unmet_requirement() {
    let env = TestEnv::new();

    // Observer hook on afterWrite fails with block policy
    let observer_rule = HookPolicyRule {
        event: "afterWrite".to_string(),
        tool: None,
        path_pattern: None,
        action: vec![
            "node".into(),
            "-e".into(),
            "console.error('linter failed'); process.exit(1)".into(),
        ],
        timeout_ms: Some(5000),
        on_failure: HookFailurePolicy::Block,
    };
    let hooks = HooksFile {
        project_trusted: true,
        rules: vec![observer_rule],
        ..Default::default()
    };

    let bus = RuntimeBus::new();
    let subscriber = HooksRuntimeSubscriber::new_with_config(
        hooks,
        HookPolicyConfig::default(),
        env.project_dir.clone(),
        env.agent_dir.clone(),
    );
    bus.subscribe(Arc::new(subscriber));

    // AfterWrite event is published. Because AfterWrite is an observer, emit_observe returns ()
    let after_write_event = RuntimeEvent::AfterWrite {
        path: env.project_dir.join("code.ts"),
        bytes: 100,
        is_error: false,
    };
    publish_observe(&bus, after_write_event);

    // But attempting BeforeCompletion must be DENIED due to unmet completion requirement
    let completion_event = RuntimeEvent::BeforeCompletion { task_id: None };
    let completion_decision = publish_decision(&bus, completion_event);
    match completion_decision {
        Err(reason) => {
            assert!(
                reason.contains("unmet completion requirements"),
                "Expected unmet completion requirements, got: {reason}"
            );
        }
        Ok(()) => panic!("Expected BeforeCompletion to be denied due to unmet requirement"),
    }
}

#[test]
fn test_before_process_start_blocking_prevents_spawn() {
    let env = TestEnv::new();

    let block_process_rule = HookPolicyRule {
        event: "beforeProcessStart".to_string(),
        tool: None,
        path_pattern: None,
        action: vec![
            "node".into(),
            "-e".into(),
            "console.error('unauthorized process command'); process.exit(1)".into(),
        ],
        timeout_ms: Some(5000),
        on_failure: HookFailurePolicy::Block,
    };
    let hooks = HooksFile {
        project_trusted: true,
        rules: vec![block_process_rule],
        ..Default::default()
    };

    let bus = RuntimeBus::new();
    let subscriber = HooksRuntimeSubscriber::new_with_config(
        hooks,
        HookPolicyConfig::default(),
        env.project_dir.clone(),
        env.agent_dir.clone(),
    );
    bus.subscribe(Arc::new(subscriber));

    let decision = publish_decision(
        &bus,
        RuntimeEvent::BeforeProcessStart {
            executable: "cmd.exe".to_string(),
            argv: vec!["/C".to_string(), "dir".to_string()],
            cwd: env.project_dir.clone(),
        },
    );

    match decision {
        Err(reason) => {
            assert!(
                reason.contains("unauthorized process command"),
                "Expected hook block reason, got: {reason}"
            );
        }
        Ok(()) => panic!("Expected BeforeProcessStart to be denied by hook"),
    }
}

#[test]
fn test_recursion_guard_limits_depth() {
    // Entering depth within limit (max_depth: 3)
    {
        let _g1 = HookDepthGuard::enter(3).expect("depth 1 should succeed");
        {
            let _g2 = HookDepthGuard::enter(3).expect("depth 2 should succeed");
            {
                let _g3 = HookDepthGuard::enter(3).expect("depth 3 should succeed");
                // 4th nesting level must be rejected
                let g4 = HookDepthGuard::enter(3);
                assert!(g4.is_err());
                assert!(g4.unwrap_err().contains("recursion depth limit exceeded"));
            }
            // After exiting depth 3, can enter again
            let _g3_again = HookDepthGuard::enter(3).expect("should succeed after release");
        }
    }
}

#[test]
fn test_bounded_streams_and_timeout() {
    let env = TestEnv::new();

    // 1. Timeout enforcement
    let timeout_rule = HookPolicyRule {
        event: "beforeWrite".to_string(),
        tool: None,
        path_pattern: None,
        action: vec![
            "node".into(),
            "-e".into(),
            "setTimeout(() => {}, 10000)".into(),
        ],
        timeout_ms: Some(300), // 300ms timeout
        on_failure: HookFailurePolicy::Block,
    };
    let initial_timed_out = GLOBAL_HOOK_TELEMETRY.timed_out.load(Ordering::Relaxed);
    let err = hooks::run_rule(
        &timeout_rule,
        "beforeWrite",
        "write",
        None,
        &json!({}),
        None,
        None,
        Some(300),
        Some(&env.project_dir),
        3,
    );
    assert!(err.is_err());
    assert!(err.unwrap_err().contains("timed out"));
    assert_eq!(
        GLOBAL_HOOK_TELEMETRY.timed_out.load(Ordering::Relaxed),
        initial_timed_out + 1
    );

    // 2. Output exceeding 64 KiB bounded stream
    let big_output_rule = HookPolicyRule {
        event: "beforeWrite".to_string(),
        tool: None,
        path_pattern: None,
        action: vec![
            "node".into(),
            "-e".into(),
            "process.stderr.write('A'.repeat(100 * 1024)); process.exit(1)".into(),
        ],
        timeout_ms: Some(5000),
        on_failure: HookFailurePolicy::Block,
    };
    let err = hooks::run_rule(
        &big_output_rule,
        "beforeWrite",
        "write",
        None,
        &json!({}),
        None,
        None,
        Some(5000),
        Some(&env.project_dir),
        3,
    );
    assert!(err.is_err());
    let msg = err.unwrap_err();
    assert!(
        msg.len() <= hooks::MAX_HOOK_STREAM_BYTES + 1024,
        "Captured error output exceeded bounded capacity: {}",
        msg.len()
    );
}

#[test]
fn test_hook_status_diagnostics() {
    let env = TestEnv::new();
    let hooks_json = json!({
        "rules": [
            {
                "event": "beforeWrite",
                "action": ["node", "-e", "process.exit(0)"],
                "onFailure": "block"
            }
        ]
    });
    env.write_project_hooks(&hooks_json.to_string());

    let report = hooks::status_report(&env.project_dir);
    assert_eq!(report["rulesCount"], 1);
    assert_eq!(report["trusted"], true);
    assert!(report["contentHash"].as_str().is_some());
    assert!(report["telemetry"]["executed"].as_u64().is_some());

    // NativeExtensionHost command dispatch
    let mut host = NativeExtensionHost::new_with_agent_dir(
        "test-session",
        &env.project_dir,
        Some(&env.agent_dir),
    );
    let cmd_result = host
        .command("hook-status", "")
        .expect("command should succeed");
    let cmd_val = cmd_result.expect("should return json value");
    assert_eq!(cmd_val["rulesCount"], 1);
    assert_eq!(cmd_val["trusted"], true);
}
