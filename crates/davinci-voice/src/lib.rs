//! Local dictation, an additive feature with no upstream TypeScript equivalent.
//! Speech contracts never own a composer, agent, submission or network client.

#[cfg(feature = "native")]
pub mod audio;
#[cfg(feature = "native")]
pub mod capture;
pub mod catalog;
#[cfg(feature = "native")]
pub mod engine;
pub mod normalize;
pub mod protocol;
pub mod state;
