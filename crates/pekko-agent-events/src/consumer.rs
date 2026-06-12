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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publisher::EventPublisher;
    use crate::schema::AgentEventEnvelope;
    use uuid::Uuid;

    fn evt(event_type: &str) -> AgentEventEnvelope {
        AgentEventEnvelope::new("test", event_type, "t1", Uuid::new_v4(), serde_json::Value::Null)
    }

    #[tokio::test]
    async fn consume_one_without_filter() {
        let pub_ = EventPublisher::new("test", 16);
        let mut consumer = EventConsumer::new(pub_.subscribe());
        pub_.publish(evt("some.event")).await.unwrap();
        let got = consumer.consume_one().await.unwrap();
        assert_eq!(got.event_type, "some.event");
    }

    #[tokio::test]
    async fn filter_skips_non_matching_prefix() {
        let pub_  = EventPublisher::new("test", 16);
        let mut consumer = EventConsumer::new(pub_.subscribe()).with_filter("workflow.");

        // agent.* event should be skipped, workflow.* should pass
        pub_.publish(evt("agent.query.started")).await.unwrap();
        pub_.publish(evt("workflow.started")).await.unwrap();

        let got = consumer.consume_one().await.unwrap();
        assert_eq!(got.event_type, "workflow.started");
    }

    #[tokio::test]
    async fn try_drain_empty_channel() {
        let pub_  = EventPublisher::new("test", 16);
        let mut consumer = EventConsumer::new(pub_.subscribe());
        assert!(consumer.try_drain().is_empty());
    }

    #[tokio::test]
    async fn try_drain_returns_pending_events() {
        let pub_  = EventPublisher::new("test", 16);
        let mut consumer = EventConsumer::new(pub_.subscribe());
        pub_.publish(evt("e.1")).await.unwrap();
        pub_.publish(evt("e.2")).await.unwrap();
        pub_.publish(evt("e.3")).await.unwrap();
        // Brief yield so the broadcast channel delivers all three
        tokio::task::yield_now().await;
        let drained = consumer.try_drain();
        assert_eq!(drained.len(), 3);
    }

    #[tokio::test]
    async fn try_drain_with_filter_skips_mismatched() {
        let pub_  = EventPublisher::new("test", 16);
        let mut consumer = EventConsumer::new(pub_.subscribe()).with_filter("saga.");
        pub_.publish(evt("saga.started")).await.unwrap();
        pub_.publish(evt("workflow.started")).await.unwrap();
        pub_.publish(evt("saga.completed")).await.unwrap();
        tokio::task::yield_now().await;
        let drained = consumer.try_drain();
        assert_eq!(drained.len(), 2);
        assert!(drained.iter().all(|e| e.event_type.starts_with("saga.")));
    }
}
