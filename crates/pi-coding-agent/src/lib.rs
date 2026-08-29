//! Embed API matching TypeScript `createAgentSession`.

pub mod sdk;
pub mod settings;

pub use sdk::{
    create_agent_session, AgentSession, CreateAgentSessionOptions, CreateAgentSessionResult,
};
