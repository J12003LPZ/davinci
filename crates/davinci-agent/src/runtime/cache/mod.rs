//! General cache mechanics. Consumers validate current dependencies and authority.
pub(crate) mod directory;
mod file;
mod identity;
mod key;
mod memory;
mod persistent;
mod singleflight;
mod telemetry;
mod workspace;
pub use file::*;
pub use identity::*;
pub use key::*;
pub use memory::*;
pub use persistent::SweepStats;
pub use singleflight::SingleFlight;
pub use telemetry::*;
pub use workspace::*;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CacheError {
    #[error("cache dependency was invalidated; refresh its version")]
    Invalidated,
    #[error("cache request denied")]
    Denied,
    #[error("cache computation cancelled")]
    Cancelled,
    #[error("cache computation panicked")]
    Panicked,
    #[error("cache waiter timed out")]
    Timeout,
    #[error("cache coordination is at capacity")]
    Busy,
    #[error("cache value type mismatch")]
    TypeMismatch,
    #[error("{0}")]
    Compute(String),
}
