//! `/setup qdrant`: the optional Qdrant projection, explained and managed.
//!
//! Qdrant is off unless the user turns it on. Nothing here installs Docker or
//! Qdrant, changes Docker Desktop, or touches a container that is not the one
//! the user recorded as DaVinci's. The default `/setup` listing shows one line
//! for it and asks Docker nothing while the backend is local.

use super::docker_discovery::{
    discover, docker_run_command_with, has_owner_label, start_consent, QdrantDiscovery,
    OWNER_LABEL, OWNER_VALUE, START_QDRANT,
};

use super::memory_backends::MemoryProjectionBackend;
use super::service_authority::ServiceConsentStore;
use super::setup::{step_from_health, SetupContext, Step, StepState};
use super::setup_profile::{load_effective_setup, OwnedContainer, SetupProfile};
use super::vector_memory::{process_env, VectorMemory};
use std::sync::{Arc, Mutex};

const ID: &str = "qdrant";
const TITLE: &str = "Qdrant projection";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QdrantCommand {
    /// Explain it and look (read-only) at what is on this machine.
    Show,
    Enable,
    Disable,
    /// Record this container (id or 12+ character prefix) as DaVinci's.
    Own(String),
    /// Forget the recorded container. It is not touched.
    Release,
    KeepRunning(bool),
}

pub fn parse(rest: &str) -> Result<QdrantCommand, String> {
    let rest = rest.trim();
    let usage =
        "use /setup qdrant, /setup qdrant enable|disable, /setup qdrant own <container-id>, \
                 /setup qdrant release or /setup qdrant keep-running on|off";
    let mut words = rest.split_whitespace();
    match (words.next(), words.next(), words.next()) {
        (None, _, _) => Ok(QdrantCommand::Show),
        (Some("enable"), None, _) => Ok(QdrantCommand::Enable),
        (Some("disable"), None, _) => Ok(QdrantCommand::Disable),
        (Some("release"), None, _) => Ok(QdrantCommand::Release),
        (Some("own"), Some(id), None) => Ok(QdrantCommand::Own(id.to_string())),
        (Some("keep-running"), Some("on"), None) => Ok(QdrantCommand::KeepRunning(true)),
        (Some("keep-running"), Some("off"), None) => Ok(QdrantCommand::KeepRunning(false)),
        _ => Err(format!("unknown /setup qdrant argument `{rest}` · {usage}")),
    }
}

/// What to read before turning it on. Plain, and honest about the cost.
pub fn explanation() -> Step {
    Step::new(
        ID,
        TITLE,
        StepState::Skipped,
        "Qdrant is an optional second copy of your memory vectors (numbers, never text). \
         Benefit: faster nearest-neighbour search on a very large store, and one index other \
         tools on this machine can share. Downsides: it needs Qdrant running (usually in Docker), \
         uses extra memory and disk, is one more service that can be down (DaVinci then answers \
         from local search and shows QDRANT_OFFLINE; nothing is lost), and on a normal-size store \
         local search is already fast. The speed-up is not measured live yet, so it stays off \
         unless you choose it. Your records always stay in the local file."
            .to_string(),
    )
    .with_action("/setup qdrant enable turns it on; /setup qdrant disable turns it off again")
}

fn recorded_id(ctx: &SetupContext) -> Option<String> {
    SetupProfile::load(ctx.agent_dir)
        .0
        .memory
        .qdrant_setup
        .owned_container
        .map(|container| container.id)
}

fn discovery(ctx: &SetupContext) -> (QdrantDiscovery, SetupProfile) {
    let effective = load_effective_setup(ctx.agent_dir, &process_env);
    let config = &effective.memory.config;
    let recorded = effective
        .profile
        .memory
        .qdrant_setup
        .owned_container
        .as_ref()
        .map(|container| container.id.clone());
    let found = discover(
        &config.qdrant_url,
        config.projection.allow_remote,
        recorded.as_deref(),
        ctx.docker,
    );
    (found, effective.profile)
}

fn discovery_step(ctx: &SetupContext) -> Step {
    let (found, profile) = discovery(ctx);
    step_from_health(
        ID,
        TITLE,
        &found.health(profile.memory.qdrant_setup.keep_running),
    )
}

/// The line in the default `/setup` listing. While the backend is local this
/// asks neither Docker nor Qdrant anything.
pub fn summary_step(ctx: &SetupContext) -> Step {
    let effective = load_effective_setup(ctx.agent_dir, &process_env);
    if effective.memory.config.projection.backend != MemoryProjectionBackend::Qdrant {
        return Step::new(
            ID,
            TITLE,
            StepState::Skipped,
            "off: memory is searched locally; /setup qdrant explains the optional Qdrant copy"
                .to_string(),
        );
    }
    discovery_step(ctx)
}

fn fail(error: String) -> Vec<Step> {
    vec![Step::new(ID, TITLE, StepState::Action, error)
        .with_action("fix the setup profile, then run the command again")]
}

fn update_profile(
    ctx: &SetupContext,
    change: impl FnOnce(&mut SetupProfile),
) -> Result<SetupProfile, String> {
    let (mut profile, _) = SetupProfile::load(ctx.agent_dir);
    change(&mut profile);
    profile.save(ctx.agent_dir)?;
    Ok(profile)
}

pub fn command(
    command: QdrantCommand,
    memory: &Arc<Mutex<VectorMemory>>,
    ctx: &SetupContext,
) -> Vec<Step> {
    match command {
        QdrantCommand::Show => vec![explanation(), discovery_step(ctx)],
        QdrantCommand::Enable | QdrantCommand::Disable => {
            let backend = if command == QdrantCommand::Enable {
                MemoryProjectionBackend::Qdrant
            } else {
                MemoryProjectionBackend::Local
            };
            if let Err(error) = update_profile(ctx, |profile| profile.memory.backend = backend) {
                return fail(error);
            }
            // Take the profile's answer and the environment's together, then
            // apply it to the running memory so no restart is needed.
            let effective = load_effective_setup(ctx.agent_dir, &process_env);
            {
                let mut live = memory.lock().unwrap_or_else(|e| e.into_inner());
                live.config.projection.backend = effective.memory.config.projection.backend;
                live.config.qdrant_url = effective.memory.config.qdrant_url.clone();
                live.config.projection.allow_remote =
                    effective.memory.config.projection.allow_remote;
            }
            if backend == MemoryProjectionBackend::Qdrant {
                let mut steps = vec![Step::new(
                    ID,
                    TITLE,
                    StepState::Done,
                    "Qdrant projection is on; records stay in the local file and local search \
                     answers whenever Qdrant does not"
                        .to_string(),
                )];
                steps.push(discovery_step(ctx));
                steps
            } else {
                vec![Step::new(
                    ID,
                    TITLE,
                    StepState::Done,
                    "Qdrant projection is off for good: no Qdrant request and no Docker call is \
                     made; the remote collection is kept as it is (delete it in Qdrant if you \
                     want it gone)"
                        .to_string(),
                )]
            }
        }
        QdrantCommand::Own(prefix) => {
            let (found, _) = discovery(ctx);
            let matches: Vec<_> = found
                .candidates
                .iter()
                .filter(|candidate| prefix.len() >= 12 && candidate.id.starts_with(&prefix))
                .collect();
            let [container] = matches.as_slice() else {
                return fail(if prefix.len() < 12 {
                    "give at least 12 characters of the container id (docker ps --no-trunc)".into()
                } else if matches.is_empty() {
                    format!("no Qdrant container starts with {prefix}; nothing was recorded")
                } else {
                    format!(
                        "{prefix} matches {} containers; give more characters",
                        matches.len()
                    )
                });
            };
            if !has_owner_label(container) {
                return fail(format!(
                    "{} does not carry the label {OWNER_LABEL}={OWNER_VALUE}, so it is not a \
                     container DaVinci's command created; nothing was recorded. Create one with: {}",
                    container.name,
                    docker_run_command_with(false)
                ));
            }
            let record = OwnedContainer {
                id: container.id.clone(),
                name: container.name.clone(),
            };
            let changed = recorded_id(ctx).as_deref() != Some(record.id.as_str());
            match update_profile(ctx, |profile| {
                profile.memory.qdrant_setup.owned_container = Some(record.clone());
            }) {
                Ok(_) => {
                    // The yes to starting a container was given for the one
                    // recorded then. A different container needs its own yes.
                    if changed {
                        if let Err(error) =
                            ServiceConsentStore::open(ctx.agent_dir).set(start_consent(), false)
                        {
                            return fail(error);
                        }
                    }
                    vec![Step::new(
                        ID,
                        TITLE,
                        StepState::Done,
                        format!(
                        "recorded {} as DaVinci's Qdrant container; it can be started only after \
                         /setup approve {START_QDRANT}, and no other container is ever touched",
                        record.name
                    ),
                    )]
                }
                Err(error) => fail(error),
            }
        }
        QdrantCommand::Release => match update_profile(ctx, |profile| {
            profile.memory.qdrant_setup.owned_container = None;
        }) {
            Ok(_) => {
                let _ = ServiceConsentStore::open(ctx.agent_dir).set(start_consent(), false);
                vec![Step::new(
                ID,
                TITLE,
                StepState::Done,
                "forgot the recorded container and withdrew the approval to start it; the container was not touched".to_string(),
            )]
            }
            Err(error) => fail(error),
        },
        QdrantCommand::KeepRunning(on) => {
            match update_profile(ctx, |profile| profile.memory.qdrant_setup.keep_running = on) {
                Ok(_) => vec![Step::new(
                    ID,
                    TITLE,
                    StepState::Done,
                    if on {
                        format!(
                            "the container command DaVinci shows now includes \
                             --restart unless-stopped; DaVinci never changes a restart policy \
                             itself: {}",
                            docker_run_command_with(true)
                        )
                    } else {
                        "the container command DaVinci shows has no restart policy".to_string()
                    },
                )],
                Err(error) => fail(error),
            }
        }
    }
}
