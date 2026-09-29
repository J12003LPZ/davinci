//! Files a user request names reach the model in that turn's appended
//! harness context. No TypeScript counterpart; see `prompt::named_files`.

use davinci_agent::turn_context::TurnContextPlacement;
use davinci_agent::PermissionRule;
use davinci_agent::{Agent, PromptProfile};
use std::fs;

fn agent(root: &std::path::Path, placement: TurnContextPlacement) -> Agent {
    let mut agent = Agent::new_builtin(PromptProfile::Stable);
    agent.named_file_context = true;
    agent.auto_compaction = false;
    agent.cwd = root.into();
    agent.provider = "openai-codex".into();
    agent.model_id = "gpt-5.6-luna".into();
    agent.turn_context_placement_override = Some(placement);
    agent
}

fn rendered(agent: &Agent) -> String {
    format!(
        "{}\n{}",
        agent.system_prompt,
        serde_json::to_string(&agent.messages_for_provider()).unwrap()
    )
}

fn turn(agent: &mut Agent, text: &str) -> String {
    agent.prompt_user_with(text, &[]);
    agent.commit_turn_context(None);
    rendered(agent)
}

#[test]
fn appended_routes_attach_the_named_file_once_outside_the_system_prompt() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("intervals.py"),
        "def merge_intervals(items):\n    return items\n",
    )
    .unwrap();
    let mut current = agent(dir.path(), TurnContextPlacement::Appended);
    let text = turn(
        &mut current,
        "Fix merge_intervals in intervals.py so touching ranges merge.",
    );
    assert_eq!(text.matches("<named_files").count(), 1, "{text}");
    assert!(text.contains("def merge_intervals(items):"));
    assert!(!current.system_prompt.contains("<named_files"));
    // A continuation of the same user turn adds nothing.
    current.commit_turn_context(None);
    assert_eq!(rendered(&current).matches("<named_files").count(), 1);
}

#[test]
fn system_prompt_routes_never_attach_so_the_prefix_stays_cached() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("calc.py"), "SECRET_MARKER = 1\n").unwrap();
    let mut plain = agent(dir.path(), TurnContextPlacement::SystemPrompt);
    turn(&mut plain, "Fix the rounding bug");
    let mut named = agent(dir.path(), TurnContextPlacement::SystemPrompt);
    let text = turn(&mut named, "Fix the rounding bug in calc.py");
    assert!(!text.contains("<named_files"), "{text}");
    assert!(!text.contains("SECRET_MARKER"), "{text}");
    assert_eq!(plain.system_prompt, named.system_prompt);
}

#[test]
fn an_unchanged_file_named_again_is_not_copied_again() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("calc.py"), "UNIQUE_BODY = 1\n").unwrap();
    let mut current = agent(dir.path(), TurnContextPlacement::Appended);
    turn(&mut current, "Fix calc.py");
    let text = turn(&mut current, "Also rename the constant in calc.py");
    assert_eq!(text.matches("UNIQUE_BODY = 1").count(), 1, "{text}");
    assert!(text.contains("calc.py: attached earlier"), "{text}");

    fs::write(dir.path().join("calc.py"), "UNIQUE_BODY = 2\n").unwrap();
    let text = turn(&mut current, "calc.py changed, look again");
    assert!(text.contains("UNIQUE_BODY = 2"), "{text}");
}

#[test]
fn queued_messages_attach_the_files_each_one_names() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("calc.py"), "CALC_BODY = 1\n").unwrap();
    fs::write(dir.path().join("util.py"), "UTIL_BODY = 1\n").unwrap();
    let mut current = agent(dir.path(), TurnContextPlacement::Appended);
    current.prompt_user_with("Fix calc.py", &[]);
    current.prompt_user_with("and util.py too", &[]);
    current.commit_turn_context(None);
    let text = rendered(&current);
    assert!(text.contains("CALC_BODY = 1"), "{text}");
    assert!(text.contains("UTIL_BODY = 1"), "{text}");
}

#[test]
fn a_turn_that_names_no_file_carries_no_block() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("calc.py"), "x = 1\n").unwrap();
    let mut current = agent(dir.path(), TurnContextPlacement::Appended);
    let first = turn(&mut current, "Fix calc.py");
    assert_eq!(first.matches("<named_files").count(), 1);
    let second = turn(&mut current, "Now explain what you changed");
    assert_eq!(second.matches("<named_files").count(), 1, "{second}");
}

#[test]
fn a_denied_read_is_never_attached() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("secret_plan.py"), "TOKEN = 'x'\n").unwrap();
    fs::write(dir.path().join("public.py"), "PUBLIC_BODY = 2\n").unwrap();
    let mut current = agent(dir.path(), TurnContextPlacement::Appended);
    current
        .permissions
        .lock()
        .unwrap()
        .deny
        .push(PermissionRule::parse("read(secret_plan.py)").unwrap());
    let text = turn(&mut current, "Compare secret_plan.py with public.py");
    assert!(text.contains("PUBLIC_BODY = 2"), "{text}");
    assert!(!text.contains("TOKEN"), "{text}");
    assert!(!text.contains("secret_plan.py ("), "{text}");
}

#[test]
fn nothing_is_read_when_off_hooked_or_without_the_read_tool() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("calc.py"), "CALC_BODY = 1\n").unwrap();
    type Setup = fn(&mut Agent);
    let setups: [(&str, Setup); 4] = [
        ("setting off", |agent| agent.named_file_context = false),
        ("hook could intercept read", |agent| {
            agent.named_file_hooks_active = true
        }),
        ("no read tool", |agent| {
            agent.tools.retain(|tool| tool != "read")
        }),
        ("worker", |agent| {
            let mut worker = davinci_agent::RuntimeHandle::new(
                davinci_agent::RunId::new(),
                davinci_agent::AgentId::new(),
                davinci_agent::RuntimeBus::new(),
            );
            worker.parent_agent_id = Some(davinci_agent::AgentId::new());
            agent.set_runtime(worker);
        }),
    ];
    for (label, setup) in setups {
        let mut current = agent(dir.path(), TurnContextPlacement::Appended);
        setup(&mut current);
        let text = turn(&mut current, "Fix calc.py");
        assert!(!text.contains("<named_files"), "{label}: {text}");
        assert!(!text.contains("CALC_BODY"), "{label}");
    }
}
