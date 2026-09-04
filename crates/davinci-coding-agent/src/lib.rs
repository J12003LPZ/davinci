//! Embed API matching TypeScript `createAgentSession`.

pub mod interactive_tui;
pub mod native_extensions;
pub mod sdk;
pub mod settings;
pub mod trust;

pub use sdk::{
    create_agent_session, AgentSession, CreateAgentSessionOptions, CreateAgentSessionResult,
    ExtensionLoadError, ExtensionManifest, LoadExtensionsResult,
};
