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
static LEGACY_EXECUTION: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
static TRANSITION: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn enable() {
    let _transition = TRANSITION.lock().unwrap_or_else(|error| error.into_inner());
    SANDBOX_ACTIVE.store(true, Ordering::SeqCst);
}

pub fn active() -> bool {
    SANDBOX_ACTIVE.load(Ordering::SeqCst)
}

/// Record before attempting an unowned spawn: a failed handshake does not
/// prove that descendants stopped. This history remains after handles drop.
pub fn record_legacy_execution(component: &'static str) -> Result<(), String> {
    let _transition = TRANSITION.lock().unwrap_or_else(|error| error.into_inner());
    require_executor(component)?;
    let _ = LEGACY_EXECUTION.set(component);
    Ok(())
}

pub fn auto_activation_blocker() -> Option<String> {
    LEGACY_EXECUTION.get().map(|component| format!(
        "{component} used legacy host execution without descendant ownership; restart directly in Auto to activate confinement"
    ))
}

/// Reserve the default boundary before publishing a policy. A concurrent raw
/// spawn must either record first (blocking Auto) or refuse after activation.
pub fn activate_auto_boundary() -> Option<String> {
    let _transition = TRANSITION.lock().unwrap_or_else(|error| error.into_inner());
    let blocker = auto_activation_blocker();
    if blocker.is_none() {
        SANDBOX_ACTIVE.store(true, Ordering::SeqCst);
    }
    blocker
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
