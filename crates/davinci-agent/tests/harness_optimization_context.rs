use std::fs;
use std::path::PathBuf;

use davinci_agent::{load_context_files_for_targets, Agent, ContextFile};
use davinci_ai::ChatMessage;

#[test]
fn root_budget_shapes_provider_view_without_dropping_authority() {
    let mut agent = Agent::new("base authority");
    agent.context_window = 30;
    agent.context_files = vec![ContextFile {
        path: PathBuf::from("AGENTS.md"),
        name: "AGENTS.md".into(),
        body: "project authority must remain".into(),
    }];
    agent.set_ephemeral_context(vec![ChatMessage::text(
        "custom",
        "optional evidence ".repeat(120),
    )]);

    let prompt = agent.provider_system_prompt();
    assert!(prompt.contains("project authority must remain"));
    let provider = agent.messages_for_provider();
    let rendered = serde_json::to_string(&provider).unwrap();
    assert!(!rendered.contains("optional evidence"));
}

#[test]
fn scoped_context_files_follow_target_ancestors() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("AGENTS.md"), "root authority").unwrap();
    fs::create_dir_all(root.join("crates/a/src")).unwrap();
    fs::create_dir_all(root.join("crates/b/src")).unwrap();
    fs::write(root.join("crates/AGENTS.md"), "crates authority").unwrap();
    fs::write(root.join("crates/b/AGENTS.md"), "unrelated b authority").unwrap();
    fs::write(root.join("crates/a/src/lib.rs"), "pub fn a() {}").unwrap();

    let files = load_context_files_for_targets(root, true, &[root.join("crates/a/src/lib.rs")]);
    let bodies = files.iter().map(|f| f.body.as_str()).collect::<Vec<_>>();
    assert!(bodies.contains(&"root authority"));
    assert!(bodies.contains(&"crates authority"));
    assert!(!bodies.contains(&"unrelated b authority"));
}
