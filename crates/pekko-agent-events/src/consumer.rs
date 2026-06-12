use tokio::sync::broadcast;
use tracing::{info, warn};

use crate::schema::AgentEventEnvelope;

/// Wraps a `broadcast::Receiver` with optional event-type filtering.
///
/// Obtain a consumer via `EventPublisher::subscribe()`.
pub struct EventConsumer {
    receiver:    broadcast::Receiver<AgentEventEnvelope>,
    filter_type: Option<String>,
}

impl EventConsumer {
    pub fn new(receiver: broadcast::Receiver<AgentEventEnvelope>) -> Self {
        Self { receiver, filter_type: None }
    }

    /// Only yield events whose `event_type` matches this prefix.
    /// E.g. `"agent."` passes `"agent.query.completed"` but not `"workflow.started"`.
    pub fn with_filter(mut self, prefix: impl Into<String>) -> Self {
        self.filter_type = Some(prefix.into());
        self
    }

    /// Receive the next matching event, skipping lagged/filtered events.
    pub async fn consume_one(&mut self) -> Option<AgentEventEnvelope> {
        loop {
            match self.receiver.recv().await {
                Ok(event) => {
                    if let Some(ref filter) = self.filter_type {
                        if !event.event_type.starts_with(filter.as_str()) {
                            continue;
                        }
                    }
                    info!(
                        event_id   = %event.event_id,
                        event_type = %event.event_type,
                        "Event consumed"
                    );
                    return Some(event);
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!(skipped = n, "Consumer lagged — skipping events");
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }

    /// Drain all currently-available events without blocking.
    pub fn try_drain(&mut self) -> Vec<AgentEventEnvelope> {
        let mut out = Vec::new();
        loop {
            match self.receiver.try_recv() {
                Ok(event) => {
                    if let Some(ref filter) = self.filter_type {
                        if !event.event_type.starts_with(filter.as_str()) {
                            continue;
                        }
                    }
                    out.push(event);
                }
                _ => break,
            }
        }
        out
    }
}
