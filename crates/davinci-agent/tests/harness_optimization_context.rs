use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use davinci_agent::{
    load_context_files_for_targets, Agent, AgentId, ContextFile, ContextItem, ContextRequest,
    ContextSource, ContextVmMode, RunId, RuntimeBus, RuntimeHandle,
};
use davinci_ai::ChatMessage;

struct RegisteredBrokerSource;

impl ContextSource for RegisteredBrokerSource {
    fn collect(&self, _request: &ContextRequest) -> Vec<ContextItem> {
        vec![ContextItem {
            source: "memory::registered-broker".into(),
            content: "broker-owned context evidence".into(),
            estimated_tokens: 5,
            priority: 900,
            stable_for_cache: true,
            provenance: serde_json::json!({
                "provenance_kind": "tool_evidence",
                "id": "registered-broker-evidence"
            }),
        }]
    }
}

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

#[test]
fn active_context_vm_reuses_repository_and_ephemeral_context_items() {
    let mut agent = Agent::new("base authority");
    agent.set_runtime(RuntimeHandle::new(
        RunId::new(),
        AgentId::new(),
        RuntimeBus::new(),
    ));
    agent.context_files = vec![ContextFile {
        path: PathBuf::from("AGENTS.md"),
        name: "AGENTS.md".into(),
        body: "repository authority".into(),
    }];
    agent.set_ephemeral_context(vec![ChatMessage::text("custom", "ephemeral evidence")]);
    agent.messages = vec![ChatMessage::text("user", "continue the task")];
    agent.set_context_vm_mode(ContextVmMode::Active);

    let image = agent.context_vm_image().unwrap();
    assert!(image
        .entries
        .iter()
        .any(|entry| entry.source_ref == "file::AGENTS.md"));
    assert!(image
        .entries
        .iter()
        .any(|entry| entry.source_ref == "ephemeral_context::0"));
}

#[test]
fn active_context_vm_reuses_registered_context_broker_items() {
    let mut agent = Agent::new("base authority");
    agent.set_runtime(RuntimeHandle::new(
        RunId::new(),
        AgentId::new(),
        RuntimeBus::new(),
    ));
    agent.register_context_source(Arc::new(RegisteredBrokerSource));
    agent.messages = vec![ChatMessage::text("user", "continue the task")];
    agent.set_context_vm_mode(ContextVmMode::Active);

    let image = agent.context_vm_image().unwrap();
    assert!(image.entries.iter().any(|entry| {
        entry.category == "broker_context"
            && entry.source_ref == "memory::registered-broker"
            && entry.content.contains("broker-owned context evidence")
    }));
}

#[test]
fn active_context_manifest_keeps_mandatory_entries_under_tight_budget() {
    let mut agent = Agent::new("base authority");
    agent.context_window = 1;
    agent.set_runtime(RuntimeHandle::new(
        RunId::new(),
        AgentId::new(),
        RuntimeBus::new(),
    ));
    agent.messages = vec![ChatMessage::text("user", "continue the task")];
    agent.set_context_vm_mode(ContextVmMode::Active);

    let manifest = agent.prepare_context_manifest("request", RunId::new(), 1, 1);
    for id in ["system_prompt", "tool_schemas"] {
        let entry = manifest
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .unwrap();
        assert!(entry.mandatory);
        assert!(entry.selected);
    }
}
