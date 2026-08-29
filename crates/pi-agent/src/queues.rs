use crate::events::AgentMessage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueMode {
    All,
    OneAtATime,
}

impl QueueMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "all" => Some(Self::All),
            "one-at-a-time" => Some(Self::OneAtATime),
            _ => None,
        }
    }
}

#[derive(Debug, Default)]
pub struct SteerQueue {
    pub items: Vec<AgentMessage>,
    pub mode: QueueMode,
}

#[derive(Debug, Default)]
pub struct FollowUpQueue {
    pub items: Vec<AgentMessage>,
    pub mode: QueueMode,
}

impl Default for QueueMode {
    fn default() -> Self {
        Self::All
    }
}

impl SteerQueue {
    pub fn enqueue(&mut self, message: AgentMessage) {
        self.items.push(message);
    }

    pub fn drain(&mut self) -> Vec<AgentMessage> {
        match self.mode {
            QueueMode::All => std::mem::take(&mut self.items),
            QueueMode::OneAtATime => {
                if self.items.is_empty() {
                    Vec::new()
                } else {
                    vec![self.items.remove(0)]
                }
            }
        }
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }
}

impl FollowUpQueue {
    pub fn enqueue(&mut self, message: AgentMessage) {
        self.items.push(message);
    }

    pub fn drain(&mut self) -> Vec<AgentMessage> {
        match self.mode {
            QueueMode::All => std::mem::take(&mut self.items),
            QueueMode::OneAtATime => {
                if self.items.is_empty() {
                    Vec::new()
                } else {
                    vec![self.items.remove(0)]
                }
            }
        }
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(s: &str) -> AgentMessage {
        AgentMessage {
            role: "user".into(),
            content: s.into(),
            images: vec![],
        }
    }

    #[test]
    fn steer_one_at_a_time() {
        let mut q = SteerQueue {
            mode: QueueMode::OneAtATime,
            items: vec![],
        };
        q.enqueue(msg("a"));
        q.enqueue(msg("b"));
        assert_eq!(q.drain().len(), 1);
        assert_eq!(q.items.len(), 1);
    }
}
