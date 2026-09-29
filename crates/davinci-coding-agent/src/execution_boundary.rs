//! Process-wide fail-closed guard for subprocess families that have not yet
//! migrated to the supervised execution-plane transport.
//!
//! A configured sandbox must never silently fall back to a legacy raw host
//! spawn. The flag is monotonic for the process: once any trusted host enables
//! execution sandboxing, every unmigrated subprocess family must either use the
//! executor transport or refuse to start. This is intentionally conservative
//! for multi-agent embedders.

use std::sync::atomic::{AtomicBool, Ordering};

static SANDBOX_ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn enable() {
    SANDBOX_ACTIVE.store(true, Ordering::SeqCst);
}

pub fn active() -> bool {
    SANDBOX_ACTIVE.load(Ordering::SeqCst)
}

pub fn require_executor(component: &str) -> Result<(), String> {
    if active() {
        Err(format!(
            "{component} requires the sandbox executor transport; refusing legacy direct host spawn"
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_sandbox_refuses_legacy_host_spawn() {
        enable();
        let error = require_executor("language server").unwrap_err();
        assert!(error.contains("refusing legacy direct host spawn"));
    }
}
