//! Opt-in, source-backed design artifacts. Session events are authoritative.
pub mod acceptance;
pub mod admission;
pub mod assets;
pub mod blob;
pub mod commands;
pub mod compile;
pub mod confinement;
pub mod context;
pub mod controller;
pub mod edits;
pub mod error;
pub mod events;
pub mod export;
pub mod generation;
pub mod handoff;
pub mod handoff_draft;
pub mod handoff_verification;
pub mod host;
pub mod interaction;
mod inventory;
pub mod model;
pub mod quality;
pub mod records;
pub mod render;
pub mod runtime;
pub mod skills;
pub mod store;
pub mod sync;
pub mod types;

pub fn enabled() -> bool {
    std::env::var("DAVINCI_DESIGN_ENABLED").as_deref() == Ok("1")
}

pub fn default_controller() -> controller::DesignController {
    controller::DesignController::new(
        store::DesignStore::new(davinci_session::default_agent_dir().join("design")),
        enabled(),
    )
}

pub fn is_command(text: &str) -> bool {
    matches!(
        text.split_whitespace().next(),
        Some("/design" | "/design-sync")
    )
}
#[cfg(test)]
mod publication_tests;
#[cfg(test)]
mod termination_tests;
