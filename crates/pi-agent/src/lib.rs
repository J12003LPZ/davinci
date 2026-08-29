//! Agent runtime matching `@earendil-works/pi-agent-core`.

pub mod compaction;
pub mod context;
pub mod events;
pub mod loop_;
pub mod permission;
pub mod queues;
pub mod skills;
pub mod templates;
pub mod tools;

pub use compaction::compact_messages;
pub use context::{
    load_context_files, load_project_context_files, render_system_prompt, resolve_prompt_input,
};
pub use events::{AgentEvent, AgentMessage};
pub use loop_::{run_agent, AgentConfig, AgentError};
pub use permission::{
    AllowAllPermissionPolicy, CallbackPermissionPolicy, DenyAllPermissionPolicy,
    NamedPermissionPolicy, PermissionDecision, PermissionPolicy, StdinAskPermissionPolicy,
};
pub use queues::{FollowUpQueue, QueueMode, SteerQueue};
pub use skills::{discover_default_skill_dirs, format_skills_for_prompt, load_skills, Skill};
pub use templates::{load_prompt_templates, PromptTemplate};
pub use tools::{
    bash, edit, find_files, grep, ls, powershell, read_file, write_file, BuiltinTool, ToolError,
    ToolRegistry, ToolResult,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingLevel {
    Off,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl ThinkingLevel {
    pub fn parse(value: &str) -> Option<Self> {
        pi_ai::ThinkingLevel::parse(value).map(|v| match v {
            pi_ai::ThinkingLevel::Off => Self::Off,
            pi_ai::ThinkingLevel::Minimal => Self::Minimal,
            pi_ai::ThinkingLevel::Low => Self::Low,
            pi_ai::ThinkingLevel::Medium => Self::Medium,
            pi_ai::ThinkingLevel::High => Self::High,
            pi_ai::ThinkingLevel::Xhigh => Self::Xhigh,
            pi_ai::ThinkingLevel::Max => Self::Max,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
        }
    }

    pub fn all() -> &'static [Self] {
        &[
            Self::Off,
            Self::Minimal,
            Self::Low,
            Self::Medium,
            Self::High,
            Self::Xhigh,
            Self::Max,
        ]
    }

    pub fn cycle(self) -> Self {
        match self {
            Self::Off => Self::Minimal,
            Self::Minimal => Self::Low,
            Self::Low => Self::Medium,
            Self::Medium => Self::High,
            Self::High => Self::Xhigh,
            Self::Xhigh => Self::Max,
            Self::Max => Self::Off,
        }
    }
}
