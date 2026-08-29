use crate::types::{AssistantMessage, AssistantMessageEvent};
use futures::Stream;
use std::pin::Pin;
use std::task::{Context as TaskContext, Poll};
use tokio::sync::mpsc;

pub struct AssistantMessageEventStream {
    receiver: mpsc::UnboundedReceiver<AssistantMessageEvent>,
    final_message: Option<AssistantMessage>,
}

impl AssistantMessageEventStream {
    pub fn new() -> (Self, mpsc::UnboundedSender<AssistantMessageEvent>) {
        let (sender, receiver) = mpsc::unbounded_channel();
        (
            Self {
                receiver,
                final_message: None,
            },
            sender,
        )
    }

    pub fn final_message(&self) -> Option<&AssistantMessage> {
        self.final_message.as_ref()
    }
}

impl Stream for AssistantMessageEventStream {
    type Item = AssistantMessageEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
        match self.receiver.poll_recv(cx) {
            Poll::Ready(Some(event)) => {
                match &event {
                    AssistantMessageEvent::Done { message, .. } => {
                        self.final_message = Some(message.clone());
                    }
                    AssistantMessageEvent::Error { error, .. } => {
                        self.final_message = Some(error.clone());
                    }
                    _ => {}
                }
                Poll::Ready(Some(event))
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}
