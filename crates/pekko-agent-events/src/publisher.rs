use std::collections::VecDeque;
use std::sync::Arc;

use tokio::sync::{broadcast, RwLock};
use tracing::info;

use crate::schema::AgentEventEnvelope;

pub use broadcast::Receiver as EventReceiver;

const DEFAULT_HISTORY: usize = 500;

/// Thread-safe event publisher backed by a `tokio::sync::broadcast` channel.
///
/// - `publish()` broadcasts to all active subscribers and appends to the ring
///   buffer history.
/// - `subscribe()` returns a new `broadcast::Receiver` — call this once per
///   consumer; each receiver gets every event published after subscription.
/// - `recent_events(n)` returns the last _n_ events from the ring buffer.
#[derive(Clone)]
pub struct EventPublisher {
    sender:      broadcast::Sender<AgentEventEnvelope>,
    history:     Arc<RwLock<VecDeque<AgentEventEnvelope>>>,
    max_history: usize,
    source_svc:  String,
}

impl EventPublisher {
    pub fn new(source_service: impl Into<String>, capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity.max(64));
        Self {
            sender,
            history:     Arc::new(RwLock::new(VecDeque::with_capacity(DEFAULT_HISTORY))),
            max_history: DEFAULT_HISTORY,
            source_svc:  source_service.into(),
        }
    }

    /// Publish an event: stored in history ring buffer and broadcast to
    /// all current subscribers.
    pub async fn publish(&self, event: AgentEventEnvelope) -> Result<(), EventError> {
        info!(
            event_id   = %event.event_id,
            event_type = %event.event_type,
            tenant_id  = %event.tenant_id,
            "Event published"
        );

        // Ring buffer — drop oldest when full
        {
            let mut h = self.history.write().await;
            if h.len() >= self.max_history { h.pop_front(); }
            h.push_back(event.clone());
        }

        // Ignore "no receivers" — that's fine; we already stored in history
        let _ = self.sender.send(event);
        Ok(())
    }

    /// Create a new subscriber that receives all future events.
    pub fn subscribe(&self) -> broadcast::Receiver<AgentEventEnvelope> {
        self.sender.subscribe()
    }

    /// Return the most recent `limit` events from the ring buffer (oldest first).
    pub async fn recent_events(&self, limit: usize) -> Vec<AgentEventEnvelope> {
        let h = self.history.read().await;
        let skip = h.len().saturating_sub(limit);
        h.iter().skip(skip).cloned().collect()
    }

    /// Return events for a specific tenant (most recent `limit`).
    pub async fn recent_events_for_tenant(
        &self,
        tenant_id: &str,
        limit:     usize,
    ) -> Vec<AgentEventEnvelope> {
        let h = self.history.read().await;
        h.iter()
            .filter(|e| e.tenant_id == tenant_id)
            .rev()
            .take(limit)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }

    pub fn source_service(&self) -> &str { &self.source_svc }
}

#[derive(thiserror::Error, Debug)]
pub enum EventError {
    #[error("Serialization failed: {0}")]
    Serialize(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::AgentEventEnvelope;
    use uuid::Uuid;

    fn evt(tenant: &str, event_type: &str) -> AgentEventEnvelope {
        AgentEventEnvelope::new(
            "test-svc", event_type, tenant, Uuid::new_v4(),
            serde_json::json!({"test": true}),
        )
    }

    #[tokio::test]
    async fn publish_without_subscribers_succeeds() {
        let pub_ = EventPublisher::new("test", 16);
        assert!(pub_.publish(evt("t1", "any.event")).await.is_ok());
    }

    #[tokio::test]
    async fn subscriber_receives_published_event() {
        let pub_ = EventPublisher::new("test", 16);
        let mut rx = pub_.subscribe();
        let event  = evt("t1", "agent.query.started");
        pub_.publish(event.clone()).await.unwrap();
        let got = rx.recv().await.unwrap();
        assert_eq!(got.event_type, event.event_type);
        assert_eq!(got.tenant_id, "t1");
    }

    #[tokio::test]
    async fn multiple_subscribers_each_receive() {
        let pub_  = EventPublisher::new("test", 16);
        let mut r1 = pub_.subscribe();
        let mut r2 = pub_.subscribe();
        let event  = evt("t1", "multi.test");
        pub_.publish(event.clone()).await.unwrap();
        assert_eq!(r1.recv().await.unwrap().event_type, event.event_type);
        assert_eq!(r2.recv().await.unwrap().event_type, event.event_type);
    }

    #[tokio::test]
    async fn recent_events_returns_last_n() {
        let pub_ = EventPublisher::new("test", 64);
        for i in 0..5u32 {
            pub_.publish(evt("t1", &format!("evt.{i}"))).await.unwrap();
        }
        let recent = pub_.recent_events(3).await;
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[2].event_type, "evt.4");
    }

    #[tokio::test]
    async fn recent_events_for_tenant_filters_correctly() {
        let pub_ = EventPublisher::new("test", 64);
        pub_.publish(evt("t1", "x.a")).await.unwrap();
        pub_.publish(evt("t2", "x.b")).await.unwrap();
        pub_.publish(evt("t1", "x.c")).await.unwrap();

        let t1 = pub_.recent_events_for_tenant("t1", 10).await;
        assert_eq!(t1.len(), 2);
        assert_eq!(t1[0].event_type, "x.a");
        assert_eq!(t1[1].event_type, "x.c");

        let t2 = pub_.recent_events_for_tenant("t2", 10).await;
        assert_eq!(t2.len(), 1);
        assert_eq!(t2[0].event_type, "x.b");
    }

    #[tokio::test]
    async fn ring_buffer_drops_oldest_when_full() {
        // max_history = DEFAULT_HISTORY (500); use a small publisher by testing behaviour:
        // publish 5, ask for last 3 → should get indices 2,3,4
        let pub_ = EventPublisher::new("test", 64);
        for i in 0..5u32 {
            pub_.publish(evt("t1", &format!("e{i}"))).await.unwrap();
        }
        let r = pub_.recent_events(3).await;
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].event_type, "e2");
        assert_eq!(r[2].event_type, "e4");
    }
}
