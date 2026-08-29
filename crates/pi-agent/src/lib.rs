//! Agent runtime matching `@earendil-works/pi-agent-core`.

pub mod compaction;
pub mod context;
pub mod events;
pub mod loop_;
pub mod queues;
pub mod skills;
pub mod templates;
pub mod tools;

pub use compaction::compact_messages;
pub use events::{AgentEvent, AgentMessage};
pub use loop_::{run_agent, AgentConfig, AgentError};
pub use queues::{FollowUpQueue, SteerQueue};
pub use tools::{
    bash, edit, read_file, write_file, BuiltinTool, ToolError, ToolRegistry, ToolResult,
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
}
