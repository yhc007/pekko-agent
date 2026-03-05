use crate::schema::AgentEventEnvelope;
use tokio::sync::broadcast;
use tracing::{info, warn};
use std::future::Future;
use std::pin::Pin;

pub type EventHandler = Box<
    dyn Fn(AgentEventEnvelope) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync,
>;

/// Event consumer that processes events from broadcast channel
pub struct EventConsumer {
    receiver: broadcast::Receiver<AgentEventEnvelope>,
    filter_type: Option<String>,
}

impl EventConsumer {
    pub fn new(receiver: broadcast::Receiver<AgentEventEnvelope>) -> Self {
        Self {
            receiver,
            filter_type: None,
        }
    }

    pub fn with_filter(mut self, event_type: impl Into<String>) -> Self {
        self.filter_type = Some(event_type.into());
        self
    }

    pub async fn consume_one(&mut self) -> Option<AgentEventEnvelope> {
        loop {
            match self.receiver.recv().await {
                Ok(event) => {
                    if let Some(ref filter) = self.filter_type {
                        if &event.event_type != filter {
                            continue;
                        }
                    }
                    info!(event_id = %event.event_id, event_type = %event.event_type, "Consumed event");
                    return Some(event);
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!(skipped = n, "Consumer lagged, skipping events");
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => {
                    return None;
                }
            }
        }
    }
}
