//! Restart recovery and the notices that go with it.
//!
//! When a session starts and the user has set the local Ollama up (consent to
//! start it, an approved executable, autostart on), a background thread asks
//! the [`ManagedServiceSupervisor`] to bring it back if it is down. The turn
//! path never waits for this and never starts anything itself.
//!
//! Notices are deduplicated by failure fingerprint (component, code, state):
//! one notice when a failure first appears, silence while it repeats, and one
//! "back again" notice when it clears. Retries back off 1s, 2s, 4s, then wait
//! 120s between attempts. A feature the user switched off gets no probe, no
//! start and no notice.

use super::service_supervisor::ManagedServiceSupervisor;
use super::setup_health::{ComponentHealth, HealthState};
use super::setup_profile::load_effective_setup;
use super::vector_memory::process_env;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Waits between attempts after a failure: 1s, 2s, 4s, then the cooldown.
const BACKOFF: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
];
/// Once the short retries are spent, one attempt per this long.
pub const COOLDOWN: Duration = Duration::from_secs(120);

/// How long to wait before attempt `failures + 1`, given `failures >= 1`
/// failed attempts so far.
pub fn backoff_delay(failures: u32) -> Duration {
    BACKOFF
        .get(failures.saturating_sub(1) as usize)
        .copied()
        .unwrap_or(COOLDOWN)
}

/// A service that is not working and needs attention.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceIncident {
    pub component: &'static str,
    pub code: Option<&'static str>,
    pub state: HealthState,
    /// Milliseconds since the Unix epoch.
    pub since: u64,
    /// What the user can do, when there is something.
    pub action: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NoticeKind {
    Incident,
    Recovered,
}

/// One line for the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceNotice {
    pub kind: NoticeKind,
    pub component: &'static str,
    pub text: String,
}

fn fingerprint(health: &ComponentHealth) -> String {
    format!("{}|{:?}|{:?}", health.component, health.code, health.state)
}

fn is_bad(state: HealthState) -> bool {
    matches!(
        state,
        HealthState::Degraded | HealthState::ActionRequired | HealthState::Failed
    )
}

/// Turns a stream of health observations into at most one notice per failure
/// and one per recovery.
#[derive(Debug, Default)]
pub struct IncidentTracker {
    open: Option<(String, ServiceIncident)>,
}

impl IncidentTracker {
    pub fn open_incident(&self) -> Option<&ServiceIncident> {
        self.open.as_ref().map(|(_, incident)| incident)
    }

    pub fn observe(&mut self, health: &ComponentHealth) -> Option<ServiceNotice> {
        if is_bad(health.state) {
            let print = fingerprint(health);
            if self.open.as_ref().is_some_and(|(open, _)| *open == print) {
                return None;
            }
            let incident = ServiceIncident {
                component: health.component,
                code: health.code,
                state: health.state,
                since: health.last_checked,
                action: health.remediation.clone(),
            };
            let mut text = format!("{}: {}", health.component, health.detail);
            if let Some(action) = &health.remediation {
                text.push_str(&format!(" {action}"));
            }
            self.open = Some((print, incident));
            return Some(ServiceNotice {
                kind: NoticeKind::Incident,
                component: health.component,
                text,
            });
        }
        if health.state == HealthState::Ready && self.open.take().is_some() {
            return Some(ServiceNotice {
                kind: NoticeKind::Recovered,
                component: health.component,
                text: format!("{} is back; semantic search is on again.", health.component),
            });
        }
        None
    }
}

/// State carried between recovery attempts.
#[derive(Debug, Default)]
pub struct RecoveryState {
    pub failures: u32,
    pub tracker: IncidentTracker,
}

#[derive(Debug)]
pub struct RecoveryRound {
    pub health: ComponentHealth,
    pub notice: Option<ServiceNotice>,
    /// When to try again; `None` means stop (ready, off, or needs the user).
    pub retry_in: Option<Duration>,
}

/// One attempt. Pure of sleeping, so it is testable without a clock.
pub fn recovery_round(
    state: &mut RecoveryState,
    supervisor: &ManagedServiceSupervisor,
    agent_dir: &Path,
) -> RecoveryRound {
    let effective = load_effective_setup(agent_dir, &process_env);
    let health = supervisor.reconcile(agent_dir, &effective);
    let notice = state.tracker.observe(&health);
    let retry_in = match health.state {
        HealthState::Ready => {
            state.failures = 0;
            None
        }
        // We tried and it did not come up: back off 1s, 2s, 4s, then cool down.
        HealthState::Failed => {
            state.failures += 1;
            Some(backoff_delay(state.failures))
        }
        // Not ours to start (autostart off, remote server): keep watching at
        // the cooldown pace so a recovery is still announced.
        HealthState::Degraded => Some(COOLDOWN),
        // Off, unconfigured, or waiting for the user: nothing to retry.
        _ => None,
    };
    RecoveryRound {
        health,
        notice,
        retry_in,
    }
}

fn queue() -> &'static Mutex<Vec<ServiceNotice>> {
    static QUEUE: std::sync::OnceLock<Mutex<Vec<ServiceNotice>>> = std::sync::OnceLock::new();
    QUEUE.get_or_init(|| Mutex::new(Vec::new()))
}

/// Notices waiting for the shell to show them. The shell polls this between
/// turns, like it polls for MCP servers that finished connecting.
pub fn drain_notices() -> Vec<ServiceNotice> {
    std::mem::take(&mut *queue().lock().unwrap_or_else(|e| e.into_inner()))
}

fn push_notice(notice: ServiceNotice) {
    queue()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(notice);
}

/// Whether the user has set the local Ollama up for automatic start. Without
/// that there is nothing to recover and nothing to announce.
pub fn recovery_wanted(agent_dir: &Path) -> bool {
    use super::service_authority::{ServiceAction, ServiceAuthorization};
    let effective = load_effective_setup(agent_dir, &process_env);
    effective.memory.enabled
        && effective.profile.memory.autostart
        && effective.profile.ollama.is_some()
        && ServiceAuthorization::check(agent_dir, ServiceAction::StartLocalOllama).is_ok()
}

/// One attempt to bring DaVinci's own Qdrant container back after a relaunch.
/// `None` health means nothing was wanted: the default setup, a disabled
/// intent, or no consent. Docker is not asked in those cases.
pub fn qdrant_recovery_round(
    agent_dir: &Path,
    docker: &dyn super::docker_discovery::DockerRunner,
    starter: &dyn super::qdrant_supervisor::ContainerStarter,
    tracker: &mut IncidentTracker,
) -> Option<(ComponentHealth, Option<ServiceNotice>)> {
    let effective = load_effective_setup(agent_dir, &process_env);
    if !super::qdrant_supervisor::start_wanted(agent_dir, &effective) {
        return None;
    }
    let health = super::qdrant_supervisor::reconcile_owned_qdrant(
        agent_dir,
        &effective,
        docker,
        starter,
        super::qdrant_supervisor::DEADLINE,
    );
    let notice = tracker.observe(&health);
    Some((health, notice))
}

/// Start the Qdrant recovery for this session: once, in the background, and
/// only if the user turned the projection on and consented to the start. When
/// Qdrant is back, the remote copy is caught up.
pub fn start_qdrant_recovery(
    agent_dir: PathBuf,
    memory: Option<Arc<Mutex<super::vector_memory::VectorMemory>>>,
) {
    let effective = load_effective_setup(&agent_dir, &process_env);
    if !super::qdrant_supervisor::start_wanted(&agent_dir, &effective) {
        // Nothing to start, but a clear confirmed in an earlier session may
        // still be owed to a Qdrant the user runs themselves. Local files
        // only unless one is owed, and then one bounded request.
        if let Some(memory) = &memory {
            super::setup_actions::finish_owed_remote_clear(memory);
        }
        return;
    }
    std::thread::spawn(move || {
        let mut tracker = IncidentTracker::default();
        let round = qdrant_recovery_round(
            &agent_dir,
            &super::docker_discovery::SystemDockerRunner,
            &super::qdrant_supervisor::SystemContainerStarter,
            &mut tracker,
        );
        let Some((health, notice)) = round else {
            return;
        };
        if let Some(notice) = notice {
            push_notice(notice);
        }
        if health.state == HealthState::Ready {
            // A clear the user confirmed is not derived-data repair: it runs
            // whatever the safe-repair setting says.
            if let Some(memory) = &memory {
                super::setup_actions::finish_owed_remote_clear(memory);
            }
            if let (true, Some(memory)) = (effective.profile.memory.safe_repair, &memory) {
                let _ = super::setup_actions::repair_step(memory);
            }
        }
    });
}

/// Start the background recovery for this session, if the user set it up.
/// Returns the stop flag; set it to end the thread.
pub fn start_recovery(
    agent_dir: PathBuf,
    supervisor: &'static ManagedServiceSupervisor,
    memory: Option<Arc<Mutex<super::vector_memory::VectorMemory>>>,
) -> Option<Arc<AtomicBool>> {
    if !recovery_wanted(&agent_dir) {
        return None;
    }
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    std::thread::spawn(move || {
        let mut state = RecoveryState::default();
        while !flag.load(Ordering::SeqCst) {
            let round = recovery_round(&mut state, supervisor, &agent_dir);
            if let Some(notice) = round.notice {
                push_notice(notice);
            }
            let Some(wait) = round.retry_in else {
                // Ollama answers: embed whatever was written while it was
                // down, if the user left safe repair on. Derived data only.
                if round.health.state == HealthState::Ready {
                    let effective = load_effective_setup(&agent_dir, &process_env);
                    if let (true, Some(memory)) = (effective.profile.memory.safe_repair, &memory) {
                        let _ = super::setup_actions::repair_step(memory);
                    }
                }
                break;
            };
            let mut waited = Duration::ZERO;
            while waited < wait && !flag.load(Ordering::SeqCst) {
                let step = Duration::from_millis(250).min(wait - waited);
                std::thread::sleep(step);
                waited += step;
            }
        }
    });
    Some(stop)
}
