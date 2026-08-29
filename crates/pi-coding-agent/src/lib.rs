//! Embed API matching TypeScript `createAgentSession`.

pub mod sdk;

pub use sdk::{
    create_agent_session, AgentSession, CreateAgentSessionOptions, CreateAgentSessionResult,
};
