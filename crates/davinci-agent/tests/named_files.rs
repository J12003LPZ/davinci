//! Files a user request names reach the model in that turn's runtime state.
//! No TypeScript counterpart; see `prompt::named_files`.

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

#[test]
fn named_file_is_attached_once_on_both_placements() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("intervals.py"),
        "def merge_intervals(items):\n    return items\n",
    )
    .unwrap();
    for placement in [
        TurnContextPlacement::SystemPrompt,
        TurnContextPlacement::Appended,
    ] {
        let mut current = agent(dir.path(), placement);
        current.prompt_user_with(
            "Fix merge_intervals in intervals.py so touching ranges merge.",
            &[],
        );
        current.commit_turn_context(None);
        let text = rendered(&current);
        assert_eq!(text.matches("<named_files>").count(), 1, "{placement:?}");
        assert!(
            text.contains("def merge_intervals(items):"),
            "{placement:?}"
        );
        // A continuation of the same user turn reuses the frozen block.
        current.commit_turn_context(None);
        assert_eq!(
            rendered(&current).matches("<named_files>").count(),
            1,
            "{placement:?}"
        );
    }
}

#[test]
fn a_turn_that_names_no_file_carries_no_block() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("calc.py"), "x = 1\n").unwrap();
    let mut current = agent(dir.path(), TurnContextPlacement::Appended);
    current.prompt_user_with("Fix calc.py", &[]);
    current.commit_turn_context(None);
    assert!(current.runtime_prompt_state().named_files.is_some());
    current.prompt_user_with("Now explain what you changed", &[]);
    assert!(current.runtime_prompt_state().named_files.is_none());
}

#[test]
fn a_denied_read_is_never_attached() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("secret_plan.py"), "TOKEN = 'x'\n").unwrap();
    fs::write(dir.path().join("public.py"), "y = 2\n").unwrap();
    let mut current = agent(dir.path(), TurnContextPlacement::Appended);
    current
        .permissions
        .lock()
        .unwrap()
        .deny
        .push(PermissionRule::parse("read(secret_plan.py)").unwrap());
    current.prompt_user_with("Compare secret_plan.py with public.py", &[]);
    let state = current.runtime_prompt_state();
    let files = state.named_files.expect("public.py is attached");
    assert_eq!(files.files.len(), 1);
    assert_eq!(files.files[0].path, "public.py");
    assert!(!davinci_agent::runtime_state_text(&current.runtime_prompt_state()).contains("TOKEN"));
}

#[test]
fn the_setting_turns_the_block_off() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("calc.py"), "x = 1\n").unwrap();
    let mut current = agent(dir.path(), TurnContextPlacement::Appended);
    current.named_file_context = false;
    current.prompt_user_with("Fix calc.py", &[]);
    current.commit_turn_context(None);
    assert!(current.runtime_prompt_state().named_files.is_none());
    assert!(!rendered(&current).contains("<named_files>"));
}
