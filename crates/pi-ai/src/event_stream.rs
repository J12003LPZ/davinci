use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::sync::mpsc;
use tokio_stream::Stream;

use crate::types::{AssistantMessage, AssistantMessageEvent};

pub struct EventStream<T, R> {
    receiver: mpsc::UnboundedReceiver<T>,
    final_result_receiver: Option<tokio::sync::oneshot::Receiver<R>>,
    final_result: Option<R>,
}

pub struct EventStreamSender<T, R> {
    sender: mpsc::UnboundedSender<T>,
    final_result_sender: Option<tokio::sync::oneshot::Sender<R>>,
}

impl<T: Send + 'static, R: Send + 'static> EventStreamSender<T, R> {
    pub fn push(&self, event: T) {
        let _ = self.sender.send(event);
    }

    pub fn end(&mut self, result: R) {
        if let Some(tx) = self.final_result_sender.take() {
            let _ = tx.send(result);
        }
    }
}

pub fn create_event_stream<T: Send + 'static, R: Send + 'static>(
) -> (EventStreamSender<T, R>, EventStream<T, R>) {
    let (sender, receiver) = mpsc::unbounded_channel();
    let (final_result_sender, final_result_receiver) = tokio::sync::oneshot::channel();
    (
        EventStreamSender {
            sender,
            final_result_sender: Some(final_result_sender),
        },
        EventStream {
            receiver,
            final_result_receiver: Some(final_result_receiver),
            final_result: None,
        },
    )
}

impl<T: Unpin, R: Unpin> Stream for EventStream<T, R> {
    type Item = T;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.receiver.poll_recv(cx)
    }
}

impl<T, R> EventStream<T, R> {
    pub async fn next_event(&mut self) -> Option<T> {
        self.receiver.recv().await
    }

    pub async fn result(mut self) -> Option<R> {
        if let Some(res) = self.final_result {
            return Some(res);
        }
        if let Some(rx) = self.final_result_receiver.take() {
            rx.await.ok()
        } else {
            None
        }
    }
}

pub type AssistantMessageEventStream = EventStream<AssistantMessageEvent, AssistantMessage>;
pub type AssistantMessageEventStreamSender =
    EventStreamSender<AssistantMessageEvent, AssistantMessage>;

pub fn create_assistant_message_event_stream() -> (
    AssistantMessageEventStreamSender,
    AssistantMessageEventStream,
) {
    create_event_stream::<AssistantMessageEvent, AssistantMessage>()
}
