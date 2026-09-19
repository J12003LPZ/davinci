//! Trusted host process supervision. No model-facing policy or process registry.
//! The caller supplies an authorized, resolved command and retains the owner.
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

mod client;
mod helper;
mod platform;
mod wire;
pub use client::Supervisor;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessConfig {
    pub executable: PathBuf,
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub environment: BTreeMap<String, String>,
}

/// Executable controlled by the host, never by a tool argument.
#[derive(Clone, Debug)]
pub struct SupervisorCommand {
    pub executable: PathBuf,
    pub argv: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessExit {
    pub code: Option<i32>,
    pub stopped: bool,
    pub error: Option<String>,
    pub output_complete: bool,
}

#[derive(Debug)]
pub enum ProcessEvent {
    Output(Vec<u8>),
    Finished(ProcessExit),
}

/// Private CLI mode: enters OS ownership before accepting a command on stdin.
pub fn run() -> ! {
    helper::run()
}
