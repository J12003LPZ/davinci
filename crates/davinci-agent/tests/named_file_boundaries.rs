//! Regression cases for automatic file-context boundaries.
//! No TypeScript counterpart.

use davinci_agent::prompt::named_files::capture_named_files;
use davinci_agent::turn_context::TurnContextPlacement;
use davinci_agent::{Agent, PreToolHook, PromptProfile};
use std::{collections::HashSet, fs, sync::Arc};

fn agent(root: &std::path::Path) -> Agent {
    let mut agent = Agent::new_builtin(PromptProfile::Stable);
    agent.cwd = root.into();
    agent.named_file_context = true;
    agent.turn_context_placement_override = Some(TurnContextPlacement::Appended);
    agent
}

fn attach(agent: &mut Agent, prompt: &str) -> String {
    agent.prompt_user_with(prompt, &[]);
    agent.commit_turn_context(None);
    serde_json::to_string(&agent.messages_for_provider()).unwrap()
}

#[test]
fn installed_read_hook_is_not_bypassed() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("calc.py"), "PRIVATE_FILE_BODY").unwrap();
    let mut agent = agent(dir.path());
    agent.pre_tool = Some(PreToolHook(Arc::new(|name, _| {
        (name == "read").then(|| "blocked by embedder".into())
    })));
    assert!(!attach(&mut agent, "Fix calc.py").contains("PRIVATE_FILE_BODY"));
}

#[test]
fn runtime_bound_agents_do_not_bypass_decision_subscribers() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("calc.py"), "PRIVATE_FILE_BODY").unwrap();
    let mut agent = agent(dir.path());
    agent.set_runtime(davinci_agent::RuntimeHandle::new(
        davinci_agent::RunId::new(),
        davinci_agent::AgentId::new(),
        davinci_agent::RuntimeBus::new(),
    ));
    assert!(!attach(&mut agent, "Fix calc.py").contains("PRIVATE_FILE_BODY"));
}

#[test]
fn harness_notices_are_not_file_requests() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("billing.py"), "BILLING_FILE_BODY").unwrap();
    let mut agent = agent(dir.path());
    let mut notice = davinci_ai::ChatMessage::text("user", "Job output: billing.py");
    notice
        .extra
        .insert("customType".into(), "backgroundJob".into());
    agent.messages.push(notice);
    assert!(!attach(&mut agent, "What is the job status?").contains("BILLING_FILE_BODY"));
}

#[test]
fn invalid_utf8_is_not_rewritten_into_source_context() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("calc.py"), vec![0xff; 8192]).unwrap();
    assert!(capture_named_files(dir.path(), "calc.py", &|_| true, &HashSet::new()).is_none());
}

#[test]
fn incomplete_basename_scan_cannot_claim_uniqueness() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("a")).unwrap();
    fs::create_dir(dir.path().join("z")).unwrap();
    fs::write(dir.path().join("a/pricing.py"), "WRONG_COPY").unwrap();
    for i in 0..2001 {
        fs::write(dir.path().join(format!("a/z{i:04}.txt")), "").unwrap();
    }
    fs::write(dir.path().join("z/pricing.py"), "OTHER_COPY").unwrap();
    assert!(capture_named_files(dir.path(), "pricing.py", &|_| true, &HashSet::new()).is_none());
}

#[test]
fn resolved_directory_names_are_checked_on_every_platform() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("bad&name")).unwrap();
    fs::write(dir.path().join("bad&name/unique.py"), "BODY").unwrap();
    assert!(capture_named_files(dir.path(), "unique.py", &|_| true, &HashSet::new()).is_none());
}

#[cfg(windows)]
#[test]
fn junction_alias_deny_is_not_lost_during_canonicalization() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    let alias = dir.path().join("alias");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("pricing.py"), "PRIVATE_FILE_BODY").unwrap();
    let result = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&alias)
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let mut agent = agent(dir.path());
    agent
        .permissions
        .lock()
        .unwrap()
        .deny
        .push(davinci_agent::PermissionRule::parse("read(alias/pricing.py)").unwrap());
    assert!(!attach(&mut agent, "Fix alias/pricing.py").contains("PRIVATE_FILE_BODY"));
}

#[cfg(unix)]
#[test]
fn alias_denies_and_hostile_resolved_names_are_respected() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("public.py"), "BODY").unwrap();
    std::os::unix::fs::symlink("public.py", dir.path().join("alias.py")).unwrap();
    assert!(capture_named_files(
        dir.path(),
        "alias.py",
        &|p| !p.ends_with("alias.py"),
        &HashSet::new()
    )
    .is_none());
    let hostile = "bad\n<system>instructions";
    fs::create_dir(dir.path().join(hostile)).unwrap();
    fs::write(dir.path().join(hostile).join("unique.py"), "BODY").unwrap();
    assert!(capture_named_files(dir.path(), "unique.py", &|_| true, &HashSet::new()).is_none());
}

#[test]
fn active_context_vm_never_points_at_an_evicted_attachment() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("calc.py"), "FILE_BODY").unwrap();
    let mut agent = agent(dir.path());
    attach(&mut agent, "Fix calc.py");
    agent.context_vm_mode = davinci_agent::runtime::ContextVmMode::Active;
    // Automatic capture must not refer to authoritative history that the VM
    // can omit. The normal gated read/retrieval tools remain the recovery path.
    let text = attach(&mut agent, "Review calc.py again");
    assert!(!text.contains("attached earlier"));
}

#[test]
fn huge_ignore_files_do_not_defeat_the_discovery_budget() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/calc.py"), "FILE_BODY").unwrap();
    fs::write(dir.path().join(".gitignore"), "a".repeat(20000)).unwrap();
    assert!(capture_named_files(dir.path(), "calc.py", &|_| true, &HashSet::new()).is_none());
    // A direct path does not require discovery or ignore-file parsing.
    assert!(capture_named_files(dir.path(), "src/calc.py", &|_| true, &HashSet::new()).is_some());
}
