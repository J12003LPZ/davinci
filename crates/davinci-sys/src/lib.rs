//! OS-facing helpers shared by every davinci crate: crash-safe file
//! publication, cross-process lock files, and supervised child processes.
//! This crate depends on no other davinci crate, so any crate can use it.

pub mod fs;
pub mod lock;
pub mod process;
