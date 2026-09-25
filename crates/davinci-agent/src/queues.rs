use davinci_ai::MessageContent;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum QueueMode {
    All,
    OneAtATime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedMessage {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub images: Vec<MessageContent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueKind {
    Steer,
    FollowUp,
}

/// Messages received by a host thread while a model turn is running.
/// The agent drains these at the next queue-injection boundary.
#[derive(Debug, Clone, Default)]
pub struct RemoteQueue(std::sync::Arc<std::sync::Mutex<Vec<(QueueKind, QueuedMessage)>>>);

impl RemoteQueue {
    pub fn push_steer(&self, text: String, images: Vec<MessageContent>) {
        self.push(QueueKind::Steer, text, images);
    }

    pub fn push_follow_up(&self, text: String, images: Vec<MessageContent>) {
        self.push(QueueKind::FollowUp, text, images);
    }

    fn push(&self, kind: QueueKind, text: String, images: Vec<MessageContent>) {
        let mut queue = self.0.lock().unwrap_or_else(|err| err.into_inner());
        queue.push((
            kind,
            QueuedMessage {
                id: uuid::Uuid::new_v4().to_string(),
                text,
                images,
            },
        ));
    }

    pub(crate) fn drain(&self) -> Vec<(QueueKind, QueuedMessage)> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(|err| err.into_inner()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SteerFollowUpQueues {
    pub steer: Vec<QueuedMessage>,
    pub follow_up: Vec<QueuedMessage>,
    pub steer_mode: QueueMode,
    pub follow_up_mode: QueueMode,
}

impl Default for SteerFollowUpQueues {
    fn default() -> Self {
        Self {
            steer: Vec::new(),
            follow_up: Vec::new(),
            steer_mode: QueueMode::All,
            follow_up_mode: QueueMode::All,
        }
    }
}

impl SteerFollowUpQueues {
    pub fn enqueue_steer(&mut self, text: impl Into<String>) {
        self.enqueue_steer_with(text, Vec::new());
    }

    pub fn enqueue_steer_with(&mut self, text: impl Into<String>, images: Vec<MessageContent>) {
        self.steer.push(QueuedMessage {
            id: uuid::Uuid::new_v4().to_string(),
            text: text.into(),
            images,
        });
    }

    pub fn enqueue_follow_up(&mut self, text: impl Into<String>) {
        self.enqueue_follow_up_with(text, Vec::new());
    }

    pub fn enqueue_follow_up_with(&mut self, text: impl Into<String>, images: Vec<MessageContent>) {
        self.follow_up.push(QueuedMessage {
            id: uuid::Uuid::new_v4().to_string(),
            text: text.into(),
            images,
        });
    }

    pub fn drain_steer(&mut self, mode: QueueMode) -> Vec<QueuedMessage> {
        drain(&mut self.steer, mode)
    }

    pub fn drain_follow_up(&mut self, mode: QueueMode) -> Vec<QueuedMessage> {
        drain(&mut self.follow_up, mode)
    }

    pub fn clear(&mut self) -> (Vec<String>, Vec<String>) {
        let steer = self.steer.drain(..).map(|m| m.text).collect();
        let follow_up = self.follow_up.drain(..).map(|m| m.text).collect();
        (steer, follow_up)
    }
}

fn drain(queue: &mut Vec<QueuedMessage>, mode: QueueMode) -> Vec<QueuedMessage> {
    match mode {
        QueueMode::All => std::mem::take(queue),
        QueueMode::OneAtATime => {
            if queue.is_empty() {
                Vec::new()
            } else {
                vec![queue.remove(0)]
            }
        }
    }
}

#[cfg(test)]
mod remote_queue_tests {
    use super::{QueueKind, RemoteQueue};

    #[test]
    fn remote_queue_preserves_kind_and_order() {
        let queue = RemoteQueue::default();
        queue.push_steer("first".into(), Vec::new());
        queue.push_follow_up("second".into(), Vec::new());
        let drained = queue.drain();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].0, QueueKind::Steer);
        assert_eq!(drained[0].1.text, "first");
        assert_eq!(drained[1].0, QueueKind::FollowUp);
        assert_eq!(drained[1].1.text, "second");
        assert!(queue.drain().is_empty());
    }
}
