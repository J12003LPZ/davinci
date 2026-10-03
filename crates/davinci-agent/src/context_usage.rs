//! What the next provider request spends its context window on, by category,
//! for `/context`. No TypeScript counterpart: pi has no `/context`; the
//! categories follow Claude Code's command of the same name.
//!
//! Every figure is the same four-bytes-a-token heuristic
//! [`Agent::estimated_context_tokens`] uses, and the categories add up to it.

use crate::Agent;
use serde::{Deserialize, Serialize};

/// One named contributor inside a category: an MCP tool, an agent profile,
/// a context file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextUsageItem {
    pub name: String,
    pub tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextUsage {
    pub model: String,
    pub context_window: u64,
    /// The system prompt without the repository context files.
    pub system_prompt: u64,
    /// Built-in, native and extension tool schemas.
    pub system_tools: u64,
    pub mcp_tools: Vec<ContextUsageItem>,
    /// Agent profiles advertised in the `agent` tool's schema.
    pub custom_agents: Vec<ContextUsageItem>,
    /// Repository context files (`AGENTS.md`, `CLAUDE.md`, …).
    pub memory_files: Vec<ContextUsageItem>,
    /// Conversation history as the provider sees it, with plan and extension
    /// context for the next request.
    pub messages: u64,
    /// The window above the auto-compaction threshold; zero when compaction
    /// is off.
    pub autocompact_buffer: u64,
}

impl ContextUsage {
    pub fn mcp_tokens(&self) -> u64 {
        self.mcp_tools.iter().map(|item| item.tokens).sum()
    }

    pub fn custom_agent_tokens(&self) -> u64 {
        self.custom_agents.iter().map(|item| item.tokens).sum()
    }

    pub fn memory_tokens(&self) -> u64 {
        self.memory_files.iter().map(|item| item.tokens).sum()
    }

    /// Everything the next request carries.
    pub fn used(&self) -> u64 {
        self.system_prompt
            + self.system_tools
            + self.mcp_tokens()
            + self.custom_agent_tokens()
            + self.memory_tokens()
            + self.messages
    }

    /// What is left before the autocompact buffer starts.
    pub fn free(&self) -> u64 {
        self.context_window
            .saturating_sub(self.used())
            .saturating_sub(self.autocompact_buffer)
    }
}

fn tokens_of(bytes: usize) -> u64 {
    (bytes as u64).div_ceil(4)
}

impl Agent {
    /// The next provider request broken down by category.
    pub fn context_usage(&self) -> ContextUsage {
        let total = self.estimated_context_tokens();

        let system_all = tokens_of(self.provider_system_prompt().len());
        let memory_files: Vec<ContextUsageItem> = self
            .context_files
            .iter()
            .map(|file| ContextUsageItem {
                name: file
                    .path
                    .strip_prefix(&self.cwd)
                    .ok()
                    .filter(|path| !path.as_os_str().is_empty())
                    .map(|path| path.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_else(|| file.path.to_string_lossy().into_owned()),
                tokens: tokens_of(file.body.len()),
            })
            .collect();
        let memory_total: u64 = memory_files.iter().map(|item| item.tokens).sum();
        let (memory_files, system_prompt) = if memory_total > system_all {
            // Deduplicated bodies are sent once; never claim more than was sent.
            (Vec::new(), system_all)
        } else {
            (memory_files, system_all - memory_total)
        };

        let specs = self.provider_tool_specs();
        let spec_tokens = |tool: &crate::AgentTool| {
            tokens_of(serde_json::to_vec(tool).map_or(0, |bytes| bytes.len()))
        };
        let tools_total = self.estimated_tool_schema_tokens();
        let mut mcp_tools: Vec<ContextUsageItem> = specs
            .iter()
            .filter(|tool| tool.name.starts_with("mcp__"))
            .map(|tool| ContextUsageItem {
                name: tool.name.clone(),
                tokens: spec_tokens(tool),
            })
            .collect();
        mcp_tools.sort_by(|a, b| b.tokens.cmp(&a.tokens).then(a.name.cmp(&b.name)));
        let custom_agents: Vec<ContextUsageItem> = if specs.iter().any(|tool| tool.name == "agent")
        {
            self.agent_profiles
                .iter()
                .map(|(name, description)| ContextUsageItem {
                    name: name.clone(),
                    tokens: tokens_of(name.len() + description.len() + 4),
                })
                .collect()
        } else {
            Vec::new()
        };
        let named_tools: u64 = mcp_tools
            .iter()
            .chain(custom_agents.iter())
            .map(|item| item.tokens)
            .sum();
        let (mcp_tools, custom_agents, system_tools) = if named_tools > tools_total {
            (Vec::new(), Vec::new(), tools_total)
        } else {
            (mcp_tools, custom_agents, tools_total - named_tools)
        };

        let messages = total.saturating_sub(system_all.saturating_add(tools_total));
        let autocompact_buffer = if self.auto_compaction {
            crate::compaction_threshold(self.context_window, &self.compaction)
                .map_or(0, |limit| self.context_window.saturating_sub(limit))
        } else {
            0
        };

        ContextUsage {
            model: self.model_id.clone(),
            context_window: self.context_window,
            system_prompt,
            system_tools,
            mcp_tools,
            custom_agents,
            memory_files,
            messages,
            autocompact_buffer,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use davinci_ai::ChatMessage;

    #[test]
    fn categories_add_up_to_the_request_estimate() {
        let mut agent = Agent::new("You are a coding agent.");
        agent.context_window = 200_000;
        agent.cwd = std::env::temp_dir();
        agent.context_files.push(crate::ContextFile {
            path: agent.cwd.join("AGENTS.md"),
            name: "AGENTS.md".into(),
            body: "Run make test before committing.".repeat(20),
        });
        agent.messages.push(ChatMessage::text(
            "user",
            "hello there, explain the runtime",
        ));
        let usage = agent.context_usage();
        assert_eq!(usage.used(), agent.estimated_context_tokens());
        assert_eq!(usage.memory_files.len(), 1);
        assert_eq!(usage.memory_files[0].name, "AGENTS.md");
        assert!(usage.messages > 0);
        assert!(usage.system_tools > 0);
        assert!(usage.autocompact_buffer >= crate::DEFAULT_RESERVE_TOKENS);
        assert_eq!(
            usage.free(),
            200_000 - usage.used() - usage.autocompact_buffer
        );
    }

    #[test]
    fn disabled_compaction_leaves_no_buffer() {
        let mut agent = Agent::new("");
        agent.context_window = 100_000;
        agent.auto_compaction = false;
        assert_eq!(agent.context_usage().autocompact_buffer, 0);
    }
}
