//! Embed API matching TypeScript `createAgentSession`.

pub mod agent_profiles;
pub mod completion_delivery;
pub mod hooks;
pub mod interaction_testing;
pub mod interactive_tui;
pub mod native_extensions;
pub mod native_tools;
pub mod prompt_host;
pub mod runtime_host;
pub mod sdk;
pub mod semantic;
pub mod settings;
pub mod trust;

pub use sdk::{
    create_agent_session, AgentSession, CreateAgentSessionOptions, CreateAgentSessionResult,
    ExtensionLoadError, ExtensionManifest, LoadExtensionsResult,
};
