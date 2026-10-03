// targeted lifecycle projection updates

use serde::{Deserialize, Serialize};

use super::{AgentId, RunId, RuntimeEvent, RuntimeEventEnvelope};

pub const CONVERSATION_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationState {
    Active,
    InTurn,
    Terminated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnOutcome {
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationIdentity {
    pub run_id: RunId,
    pub agent_id: AgentId,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationSnapshot {
    pub schema_version: u16,
    pub identity: ConversationIdentity,
    pub state: ConversationState,
    pub prompt_count: u64,
    pub turn_count: u64,
    pub active_turn: Option<u64>,
    pub last_outcome: Option<TurnOutcome>,
    #[serde(default)]
    turn_failure_pending: bool,
    pub terminated_reason: Option<String>,
    pub last_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationError {
    WrongRun,
    WrongSession,
    WrongAgent,
    OutOfOrder {
        expected: u64,
        found: u64,
    },
    InvalidTransition {
        state: ConversationState,
        event: &'static str,
    },
    MissingActiveTurn,
    UnsupportedSchema(u16),
}

impl std::fmt::Display for ConversationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "conversation event rejected: {self:?}")
    }
}
impl std::error::Error for ConversationError {}

#[derive(Debug, Clone)]
pub struct ConversationRuntime {
    snapshot: ConversationSnapshot,
}

impl ConversationRuntime {
    pub fn new(run_id: RunId, agent_id: AgentId, session_id: Option<String>) -> Self {
        Self {
            snapshot: ConversationSnapshot {
                schema_version: CONVERSATION_SCHEMA_VERSION,
                identity: ConversationIdentity {
                    run_id,
                    agent_id,
                    session_id,
                },
                state: ConversationState::Active,
                prompt_count: 0,
                turn_count: 0,
                active_turn: None,
                last_outcome: None,
                turn_failure_pending: false,
                terminated_reason: None,
                last_sequence: 0,
            },
        }
    }

    pub fn snapshot(&self) -> ConversationSnapshot {
        self.snapshot.clone()
    }

    pub fn set_session_id(&mut self, session_id: Option<String>) {
        self.snapshot.identity.session_id = session_id;
    }

    pub fn mark_turn_failed(&mut self) -> Result<(), ConversationError> {
        if self.snapshot.state != ConversationState::InTurn {
            return Err(ConversationError::MissingActiveTurn);
        }
        self.snapshot.turn_failure_pending = true;
        Ok(())
    }

    pub fn apply(&mut self, envelope: &RuntimeEventEnvelope) -> Result<(), ConversationError> {
        if envelope.schema_version > CONVERSATION_SCHEMA_VERSION {
            return Err(ConversationError::UnsupportedSchema(
                envelope.schema_version,
            ));
        }
        if envelope.run_id != self.snapshot.identity.run_id {
            return Err(ConversationError::WrongRun);
        }
        if envelope.agent_id != Some(self.snapshot.identity.agent_id) {
            return Err(ConversationError::WrongAgent);
        }
        if envelope.session_id != self.snapshot.identity.session_id {
            return Err(ConversationError::WrongSession);
        }
        if envelope.sequence <= self.snapshot.last_sequence {
            return Err(ConversationError::OutOfOrder {
                expected: self.snapshot.last_sequence + 1,
                found: envelope.sequence,
            });
        }

        let name = match &envelope.payload {
            RuntimeEvent::SessionStarted { .. } => "session_started",
            RuntimeEvent::SessionEnded { .. } => "session_ended",
            RuntimeEvent::UserPromptSubmitted => "user_prompt_submitted",
            RuntimeEvent::TurnStarted => "turn_started",
            RuntimeEvent::TurnEnded { .. } => "turn_ended",
            _ => {
                self.snapshot.last_sequence = envelope.sequence;
                return Ok(());
            }
        };
        match &envelope.payload {
            RuntimeEvent::SessionStarted { .. }
                if self.snapshot.state == ConversationState::Terminated =>
            {
                return Err(ConversationError::InvalidTransition {
                    state: self.snapshot.state,
                    event: name,
                })
            }
            RuntimeEvent::SessionStarted { .. } => {}
            RuntimeEvent::UserPromptSubmitted
                if matches!(
                    self.snapshot.state,
                    ConversationState::Active | ConversationState::InTurn
                ) =>
            {
                self.snapshot.prompt_count += 1
            }
            RuntimeEvent::UserPromptSubmitted => {
                return Err(ConversationError::InvalidTransition {
                    state: self.snapshot.state,
                    event: name,
                })
            }
            RuntimeEvent::TurnStarted if self.snapshot.state == ConversationState::Active => {
                self.snapshot.turn_count += 1;
                self.snapshot.active_turn = Some(self.snapshot.turn_count);
                self.snapshot.last_outcome = None;
                self.snapshot.turn_failure_pending = false;
                self.snapshot.state = ConversationState::InTurn;
            }
            RuntimeEvent::TurnStarted => {
                return Err(ConversationError::InvalidTransition {
                    state: self.snapshot.state,
                    event: name,
                })
            }
            RuntimeEvent::TurnEnded { success }
                if self.snapshot.state == ConversationState::InTurn =>
            {
                self.snapshot.last_outcome = Some(if *success {
                    TurnOutcome::Succeeded
                } else if self.snapshot.turn_failure_pending {
                    TurnOutcome::Failed
                } else {
                    TurnOutcome::Cancelled
                });
                self.snapshot.turn_failure_pending = false;
                self.snapshot.active_turn = None;
                self.snapshot.state = ConversationState::Active;
            }
            RuntimeEvent::TurnEnded { .. } => return Err(ConversationError::MissingActiveTurn),
            RuntimeEvent::SessionEnded { reason }
                if self.snapshot.state == ConversationState::Active =>
            {
                self.snapshot.state = ConversationState::Terminated;
                self.snapshot.terminated_reason = Some(reason.clone());
            }
            RuntimeEvent::SessionEnded { .. } => {
                return Err(ConversationError::InvalidTransition {
                    state: self.snapshot.state,
                    event: name,
                })
            }
            _ => unreachable!(),
        }
        self.snapshot.last_sequence = envelope.sequence;
        Ok(())
    }

    pub fn replay(
        identity: ConversationIdentity,
        envelopes: &[RuntimeEventEnvelope],
    ) -> Result<Self, ConversationError> {
        let mut runtime = Self::new(identity.run_id, identity.agent_id, identity.session_id);
        for envelope in envelopes {
            runtime.apply(envelope)?;
        }
        Ok(runtime)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn event(
        seq: u64,
        run: RunId,
        session: &str,
        agent: AgentId,
        payload: RuntimeEvent,
    ) -> RuntimeEventEnvelope {
        RuntimeEventEnvelope {
            schema_version: 1,
            event_id: Uuid::nil(),
            sequence: seq,
            timestamp_ms: 0,
            run_id: run,
            session_id: Some(session.into()),
            agent_id: Some(agent),
            parent_agent_id: None,
            payload,
        }
    }

    #[test]
    fn transitions_and_replay_are_deterministic() {
        let run = RunId::new();
        let agent = AgentId::new();
        let events = vec![
            event(1, run, "s", agent, RuntimeEvent::UserPromptSubmitted),
            event(2, run, "s", agent, RuntimeEvent::TurnStarted),
            event(
                3,
                run,
                "s",
                agent,
                RuntimeEvent::TurnEnded { success: true },
            ),
        ];
        let mut a = ConversationRuntime::new(run, agent, Some("s".into()));
        for e in &events {
            a.apply(e).unwrap();
        }
        let b = ConversationRuntime::replay(
            ConversationIdentity {
                run_id: run,
                agent_id: agent,
                session_id: Some("s".into()),
            },
            &events,
        )
        .unwrap();
        assert_eq!(a.snapshot(), b.snapshot());
        assert_eq!(a.snapshot().prompt_count, 1);
        assert_eq!(a.snapshot().last_outcome, Some(TurnOutcome::Succeeded));
    }

    #[test]
    fn rejects_invalid_and_mismatched_events() {
        let run = RunId::new();
        let agent = AgentId::new();
        let mut c = ConversationRuntime::new(run, agent, Some("s".into()));
        let bad = event(
            1,
            run,
            "s",
            agent,
            RuntimeEvent::TurnEnded { success: true },
        );
        assert!(matches!(
            c.apply(&bad),
            Err(ConversationError::MissingActiveTurn)
        ));
        let wrong = event(
            1,
            RunId::new(),
            "s",
            agent,
            RuntimeEvent::UserPromptSubmitted,
        );
        assert!(matches!(c.apply(&wrong), Err(ConversationError::WrongRun)));
    }
}
