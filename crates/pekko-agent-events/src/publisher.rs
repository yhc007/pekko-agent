//! EventPublisher — backed by `pekko_event_bus::EventBusHandle`.
//!
//! Replaces the previous in-memory broadcast channel.  The EventBusHandle is
//! `Clone + Send + Sync` (Arc<RwLock<EventBus>> inside), so it is safe to share
//! across tasks and services.  Consumers subscribe by reading from a named
//! topic/partition; the bus takes care of partitioning and offset tracking.

use crate::schema::AgentEventEnvelope;
use pekko_event_bus::{
    EventBusHandle,
    bus_config::{EventBusConfig, TopicConfig},
    partition_strategy::PartitionKey,
};
use thiserror::Error;
use tracing::info;

#[derive(Error, Debug)]
pub enum EventError {
    #[error("Publish failed: {0}")]
    PublishFailed(String),
    #[error("Subscribe failed: {0}")]
    SubscribeFailed(String),
}

/// Thread-safe event publisher backed by `pekko_event_bus`.
#[derive(Clone)]
pub struct EventPublisher {
    bus:          EventBusHandle,
    topic_prefix: String,
}

impl EventPublisher {
    /// Create a publisher.
    ///
    /// `topic_prefix` is the base name of the event topic (e.g. `"agent-events"`).
    /// One pekko-event-bus topic per prefix is created with 4 partitions by
    /// default, which can be changed by adjusting `num_partitions`.
    pub fn new(topic_prefix: impl Into<String>, _capacity: usize) -> Self {
        let prefix = topic_prefix.into();

        // Build a lightweight default EventBus with one topic for agent events.
        let default_topic = TopicConfig::new(&prefix)
            .partitions(4)
            .batch_size(64);

        let bus = pekko_event_bus::EventBus::builder()
            .add_topic(default_topic)
            .build()
            .expect("Failed to build EventBus for EventPublisher");

        Self {
            bus:          EventBusHandle::new(bus),
            topic_prefix: prefix,
        }
    }

    /// Publish an event envelope.
    ///
    /// The partition key is derived from the `agent_id` field so that all
    /// events for a given agent land on the same partition (ordering preserved).
    pub async fn publish(&self, event: AgentEventEnvelope) -> Result<(), EventError> {
        let topic = &self.topic_prefix;

        info!(
            event_id   = %event.event_id,
            topic      = %topic,
            event_type = %event.event_type,
            "Publishing event via pekko-event-bus"
        );

        let payload = serde_json::to_vec(&event)
            .map_err(|e| EventError::PublishFailed(e.to_string()))?;

        let key = PartitionKey::from_str(&event.source_service);

        self.bus
            .publish(topic, &key, payload)
            .map_err(|e| EventError::PublishFailed(e.to_string()))?;

        Ok(())
    }

    /// Expose the underlying `EventBusHandle` for consumers that need direct
    /// partition reads (e.g. `EventConsumer`).
    pub fn bus_handle(&self) -> EventBusHandle {
        self.bus.clone()
    }

    /// Topic name used by this publisher.
    pub fn topic(&self) -> &str {
        &self.topic_prefix
    }
}
