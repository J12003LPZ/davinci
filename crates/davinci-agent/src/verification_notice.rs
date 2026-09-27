//! Completion evidence belongs to host display events, not model-authored chat.

use crate::{Agent, AgentEvent, CompletionEvidence};

impl Agent {
    pub(crate) fn emit_verification_notice(&mut self, events: &mut Vec<AgentEvent>) {
        let status = self.completion_evidence();
        let text = match status {
            CompletionEvidence::Partial => "Verification is partial: passing checks cover some changes; other changed paths remain unchecked.",
            CompletionEvidence::Unverified => "Verification is incomplete: no applicable completed check confirms the latest changes.",
            CompletionEvidence::VerificationFailed => "Verification failed after the latest changes; the failure remains unresolved.",
            _ => { self.last_verification_notice = None; return; }
        };
        let generation = self.mutation_verification_state().mutation_generation;
        if self.last_verification_notice == Some((generation, status)) {
            return;
        }
        self.last_verification_notice = Some((generation, status));
        self.push_event(
            events,
            AgentEvent::VerificationNotice {
                status,
                generation,
                text: text.into(),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use davinci_ai::{AssistantMessage, ContentBlock, StopReason};

    fn reply(_: &Agent) -> Result<AssistantMessage, String> {
        Ok(AssistantMessage {
            id: "answer".into(),
            role: "assistant".into(),
            content: vec![ContentBlock::Text {
                text: "The model's complete answer.".into(),
            }],
            model: "fixture".into(),
            usage: None,
            stop_reason: Some(StopReason::Stop),
            error_message: None,
        })
    }

    #[test]
    fn notice_preserves_final_answer_provider_history_and_saved_session() {
        let dir = tempfile::tempdir().unwrap();
        let mut agent = Agent::new("test");
        agent.auto_compaction = false;
        agent.auto_verify = false;
        agent.session = Some(crate::JsonlSession::create(dir.path(), "notice", None).unwrap());
        agent.prompt_user_with("Make the change", &[]);
        agent.record_successful_mutation_paths(vec!["app.py".into()]);
        let events = agent.run_loop(reply).unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::VerificationNotice {
                status: CompletionEvidence::Unverified,
                ..
            }
        )));
        assert_eq!(
            agent.last_assistant_text().as_deref(),
            Some("The model's complete answer.")
        );
        let AgentEvent::AgentEnd { messages, .. } = events.last().unwrap() else {
            panic!("missing final event");
        };
        assert_eq!(
            davinci_ai::content_text(&messages.last().unwrap().content),
            "The model's complete answer."
        );
        for text in [
            serde_json::to_string(&agent.messages_for_provider()).unwrap(),
            std::fs::read_to_string(&agent.session.as_ref().unwrap().path).unwrap(),
        ] {
            assert!(!text.contains("Verification is incomplete"));
            assert!(!text.contains("davinciVerificationStatus"));
        }
        agent.prompt_user_with("Explain that answer", &[]);
        let events = agent.run_loop(reply).unwrap();
        assert!(!events
            .iter()
            .any(|event| matches!(event, AgentEvent::VerificationNotice { .. })));
        assert_eq!(
            agent.last_assistant_text().as_deref(),
            Some("The model's complete answer.")
        );
    }

    #[test]
    fn restored_legacy_notice_cannot_replace_or_replay_as_the_model_answer() {
        let dir = tempfile::tempdir().unwrap();
        let mut old = Agent::new("test");
        old.session = Some(crate::JsonlSession::create(dir.path(), "old-notice", None).unwrap());
        old.record_assistant("Saved model answer");
        let mut notice = davinci_ai::ChatMessage::text("assistant", "Legacy verification notice");
        notice.extra.insert(
            "davinciVerificationStatus".into(),
            serde_json::json!("unverified"),
        );
        old.session
            .as_mut()
            .unwrap()
            .append_entry(crate::chat_entry(
                "assistant",
                serde_json::to_value(&notice.content).unwrap(),
                &notice.extra,
            ))
            .unwrap();
        old.messages.push(notice);
        assert_eq!(
            old.last_assistant_text().as_deref(),
            Some("Saved model answer")
        );
        let path = old.session.as_ref().unwrap().path.clone();
        drop(old);
        let mut resumed = Agent::new("test");
        resumed
            .load_from_session(crate::JsonlSession::open(&path).unwrap())
            .unwrap();
        assert_eq!(
            resumed.last_assistant_text().as_deref(),
            Some("Saved model answer")
        );
        assert!(!serde_json::to_string(&resumed.messages_for_provider())
            .unwrap()
            .contains("Legacy verification notice"));
        assert!(std::fs::read_to_string(path)
            .unwrap()
            .contains("Legacy verification notice"));
    }

    #[test]
    fn changed_evidence_can_emit_a_new_notice_without_chat_messages() {
        let mut agent = Agent::new("test");
        agent.record_successful_mutation_paths(vec!["app.py".into()]);
        let mut events = Vec::new();
        agent.emit_verification_notice(&mut events);
        agent.emit_verification_notice(&mut events);
        assert_eq!(events.len(), 1);
        agent.record_verification_result(false);
        agent.emit_verification_notice(&mut events);
        assert!(matches!(
            events.last(),
            Some(AgentEvent::VerificationNotice {
                status: CompletionEvidence::VerificationFailed,
                ..
            })
        ));
        agent.record_successful_mutation_paths(vec!["app.py".into()]);
        agent.emit_verification_notice(&mut events);
        assert_eq!(events.len(), 3);
        assert!(agent.messages.is_empty());
        assert_eq!(events[0].kind(), "verification_notice");
        let encoded = serde_json::to_value(&events[0]).unwrap();
        assert_eq!(encoded["type"], "verification_notice");
        assert!(encoded.get("message").is_none());
    }
}
