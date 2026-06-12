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
