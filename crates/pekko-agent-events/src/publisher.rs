use crate::schema::AgentEventEnvelope;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tracing::info;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum EventError {
    #[error("Publish failed: {0}")]
    PublishFailed(String),
    #[error("Subscribe failed: {0}")]
    SubscribeFailed(String),
}

/// In-memory event publisher (production: Kafka FutureProducer)
pub struct EventPublisher {
    sender: broadcast::Sender<AgentEventEnvelope>,
    history: Arc<RwLock<Vec<AgentEventEnvelope>>>,
    topic_prefix: String,
}

impl EventPublisher {
    pub fn new(topic_prefix: impl Into<String>, capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self {
            sender,
            history: Arc::new(RwLock::new(Vec::new())),
            topic_prefix: topic_prefix.into(),
        }
    }

    pub async fn publish(&self, event: AgentEventEnvelope) -> Result<(), EventError> {
        let topic = format!("{}.{}", self.topic_prefix, event.event_type);
        info!(
            event_id = %event.event_id,
            topic = %topic,
            event_type = %event.event_type,
            "Publishing event"
        );

        let mut history = self.history.write().await;
        history.push(event.clone());

        let _ = self.sender.send(event);
        Ok(())
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AgentEventEnvelope> {
        self.sender.subscribe()
    }

    pub async fn get_history(&self) -> Vec<AgentEventEnvelope> {
        self.history.read().await.clone()
    }
}
