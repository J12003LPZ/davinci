//! Embed API matching TypeScript `createAgentSession`.

pub mod agent_profiles;
pub mod args;
pub mod completion_delivery;
pub mod decision_providers;
pub mod decision_state;
pub mod hooks;
pub mod interaction_testing;
pub mod interactive_tui;
pub mod native_extensions;
pub mod native_tools;
pub mod optimization;
pub mod package_source;
pub mod permissions;
pub mod plugins;
pub mod project_config;
pub mod prompt_host;
pub mod runtime_host;
pub mod runtime_inspect;
pub mod sdk;
pub mod self_update;
pub mod semantic;
pub mod settings;
pub mod trust;

pub use sdk::{
    create_agent_session, AgentSession, CreateAgentSessionOptions, CreateAgentSessionResult,
    ExtensionLoadError, ExtensionManifest, LoadExtensionsResult,
};
