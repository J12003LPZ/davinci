#!/usr/bin/env python3
from __future__ import annotations

import argparse
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one match, found {count}")
    return text.replace(old, new, 1)


def add_tests() -> None:
    path = ROOT / "crates/davinci-agent/tests/harness_optimization_context.rs"
    if path.exists():
        return
    path.write_text(r'''use std::fs;
use std::path::PathBuf;

use davinci_agent::{
    load_context_files_for_targets, Agent, ContextFile,
};
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

    let files = load_context_files_for_targets(
        root,
        true,
        &[root.join("crates/a/src/lib.rs")],
    );
    let bodies = files.iter().map(|f| f.body.as_str()).collect::<Vec<_>>();
    assert!(bodies.contains(&"root authority"));
    assert!(bodies.contains(&"crates authority"));
    assert!(!bodies.contains(&"unrelated b authority"));
}
''')


def apply_impl() -> None:
    context_path = ROOT / "crates/davinci-agent/src/context.rs"
    text = context_path.read_text()
    text = replace_once(
        text,
        "use std::collections::HashMap;",
        "use std::collections::{HashMap, HashSet};\n\nuse davinci_ai::ChatMessage;",
        "context imports",
    )
    insertion = r'''

#[derive(Debug, Clone)]
pub struct SelectedRootContext {
    pub report: ContextBudgetReport,
    pub repository_files: Vec<ContextFile>,
    pub ephemeral_messages: Vec<ChatMessage>,
}
'''
    text = replace_once(
        text,
        "pub struct ContextBudgetReport {\n    pub contributions: Vec<ContextContribution>,\n    pub total_estimated_tokens: u64,\n}\n",
        "pub struct ContextBudgetReport {\n    pub contributions: Vec<ContextContribution>,\n    pub total_estimated_tokens: u64,\n}\n" + insertion,
        "selected root context type",
    )
    scoped = r'''

pub fn load_context_files_for_targets(
    cwd: &Path,
    enabled: bool,
    targets: &[PathBuf],
) -> Vec<ContextFile> {
    if !enabled {
        return Vec::new();
    }

    let root = fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let mut files = load_context_files(cwd, true);
    let mut seen = HashSet::new();
    for file in &files {
        seen.insert(fs::canonicalize(&file.path).unwrap_or_else(|_| file.path.clone()));
    }

    for target in targets {
        let absolute = if target.is_absolute() {
            target.clone()
        } else {
            cwd.join(target)
        };
        let start = if absolute.is_dir() {
            absolute
        } else {
            absolute.parent().unwrap_or(cwd).to_path_buf()
        };
        let start = fs::canonicalize(&start).unwrap_or(start);
        if !start.starts_with(&root) {
            continue;
        }
        let mut ancestors = start
            .ancestors()
            .take_while(|path| path.starts_with(&root))
            .map(Path::to_path_buf)
            .collect::<Vec<_>>();
        ancestors.reverse();
        for directory in ancestors {
            for name in ["AGENTS.md", "CLAUDE.md"] {
                let path = directory.join(name);
                if !path.is_file() {
                    continue;
                }
                let identity = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
                if !seen.insert(identity) {
                    continue;
                }
                if let Ok(body) = fs::read_to_string(&path) {
                    files.push(ContextFile {
                        path,
                        name: name.to_string(),
                        body,
                    });
                }
            }
        }
    }
    files
}
'''
    marker = "\n#[cfg(test)]\nmod tests {"
    if "pub fn load_context_files_for_targets(" not in text:
        if marker not in text:
            raise SystemExit("context test marker missing")
        text = text.replace(marker, scoped + marker, 1)
    context_path.write_text(text)

    lib_path = ROOT / "crates/davinci-agent/src/lib.rs"
    lib = lib_path.read_text()
    lib = replace_once(
        lib,
        "    load_context_files, ContextBudgetReport, ContextContribution, ContextFile, ContextPriority,\n    RootContextAccount,\n",
        "    load_context_files, load_context_files_for_targets, ContextBudgetReport, ContextContribution,\n    ContextFile, ContextPriority, RootContextAccount, SelectedRootContext,\n",
        "context reexports",
    )

    old_messages = r'''    pub fn messages_for_provider(&self) -> Vec<ChatMessage> {
        let plan_context = self
            .plan_provider_context()
            .map(|text| ChatMessage::text("custom", text));
        if self.ephemeral_context.is_empty()
            && self.pruned_tool_results.is_empty()
            && plan_context.is_none()
        {
            return convert_to_llm_for_provider(&self.messages, self.block_images);
        }
        let mut messages = self.project_with_evidence();
        if self.ephemeral_context.is_empty() && plan_context.is_none() {
            return convert_to_llm_for_provider(&messages, self.block_images);
        }
        let insertion = messages
            .iter()
            .rposition(|message| message.role == "user")
            .unwrap_or(messages.len());
        messages.splice(
            insertion..insertion,
            plan_context
                .into_iter()
                .chain(self.ephemeral_context.iter().cloned()),
        );
        convert_to_llm_for_provider(&messages, self.block_images)
    }
'''
    new_messages = r'''    pub fn messages_for_provider(&self) -> Vec<ChatMessage> {
        let plan_context = self
            .plan_provider_context()
            .map(|text| ChatMessage::text("custom", text));
        let selected = self.select_root_context(self.context_window);
        let ephemeral_context = selected.ephemeral_messages;
        if ephemeral_context.is_empty()
            && self.pruned_tool_results.is_empty()
            && plan_context.is_none()
        {
            return convert_to_llm_for_provider(&self.messages, self.block_images);
        }
        let mut messages = self.project_with_evidence();
        if ephemeral_context.is_empty() && plan_context.is_none() {
            return convert_to_llm_for_provider(&messages, self.block_images);
        }
        let insertion = messages
            .iter()
            .rposition(|message| message.role == "user")
            .unwrap_or(messages.len());
        messages.splice(
            insertion..insertion,
            plan_context.into_iter().chain(ephemeral_context),
        );
        convert_to_llm_for_provider(&messages, self.block_images)
    }
'''
    lib = replace_once(lib, old_messages, new_messages, "provider message projection")

    lib = replace_once(
        lib,
        "            + estimate_context_tokens(&self.ephemeral_context)\n",
        "            + estimate_context_tokens(\n                &self.select_root_context(self.context_window).ephemeral_messages,\n            )\n",
        "estimated selected ephemeral context",
    )

    start = lib.index("    /// Account for the normal/root provider request without changing its prompt.\n    pub fn root_context_budget_report")
    end = lib.index("\n    pub fn apply_extension_tools", start)
    replacement = r'''    /// Account for the normal/root provider request and select optional request-local context.
    pub fn root_context_budget_report(&self, budget: u64) -> ContextBudgetReport {
        let mut account = RootContextAccount::default();
        account.add(
            "system_prompt",
            self.system_prompt.clone(),
            true,
            ContextPriority::Mandatory,
        );
        if let Some(suffix) = self.provider_system_prompt_suffix.as_ref() {
            account.add(
                "provider_system_prompt_suffix",
                suffix.clone(),
                false,
                ContextPriority::Mandatory,
            );
        }
        for file in &self.context_files {
            account.add(
                format!("repository_instruction::{}", file.path.display()),
                file.body.clone(),
                true,
                ContextPriority::Mandatory,
            );
        }
        account.add(
            "provider_tool_schemas",
            serde_json::to_string(&self.provider_tool_schema_value()).unwrap_or_default(),
            true,
            ContextPriority::Mandatory,
        );
        if let Some(contract) = self
            .tool_context
            .active_contract
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
        {
            account.add(
                "active_task_contract",
                serde_json::to_string(contract).unwrap_or_default(),
                false,
                ContextPriority::Mandatory,
            );
        }
        if let Some(plan) = self.plan_provider_context() {
            account.add("living_plan", plan, false, ContextPriority::Mandatory);
        }

        // Current extension/memory evidence gets first claim on Important space.
        for (index, message) in self.ephemeral_context.iter().enumerate() {
            account.add(
                format!("ephemeral_context::{index}"),
                serde_json::to_string(message).unwrap_or_default(),
                false,
                ContextPriority::Important,
            );
        }

        let latest_user = self.messages.iter().rposition(|message| message.role == "user");
        for (index, message) in self.messages.iter().enumerate() {
            let priority = if Some(index) == latest_user {
                ContextPriority::Mandatory
            } else {
                ContextPriority::Important
            };
            account.add(
                format!("conversation::{index}"),
                serde_json::to_string(message).unwrap_or_default(),
                false,
                priority,
            );
        }
        for skill in &self.skills {
            account.add(
                format!("skill::{}", skill.name),
                skill.body.clone(),
                true,
                ContextPriority::Deferred,
            );
        }
        for template in &self.templates {
            account.add(
                format!("prompt_template::{}", template.name),
                template.body.clone(),
                true,
                ContextPriority::Deferred,
            );
        }
        account.report_for_budget(budget)
    }

    pub fn select_root_context(&self, budget: u64) -> SelectedRootContext {
        let report = self.root_context_budget_report(budget);
        let selected_sources = report
            .contributions
            .iter()
            .filter(|entry| entry.selected)
            .map(|entry| entry.source.as_str())
            .collect::<std::collections::HashSet<_>>();
        let ephemeral_messages = self
            .ephemeral_context
            .iter()
            .enumerate()
            .filter(|(index, _)| {
                selected_sources.contains(format!("ephemeral_context::{index}").as_str())
            })
            .map(|(_, message)| message.clone())
            .collect();
        SelectedRootContext {
            report,
            repository_files: self.context_files.clone(),
            ephemeral_messages,
        }
    }
'''
    lib = lib[:start] + replacement + lib[end:]
    lib_path.write_text(lib)


def main() -> None:
    parser = argparse.ArgumentParser()
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--tests", action="store_true")
    mode.add_argument("--impl", action="store_true")
    args = parser.parse_args()
    if args.tests:
        add_tests()
    else:
        apply_impl()


if __name__ == "__main__":
    main()
