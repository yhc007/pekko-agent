use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};
use crate::error::MemoryError;
use crate::message::Message;
use std::collections::HashMap;

/// Short-term memory (conversation context)
#[async_trait]
pub trait ShortTermMemory: Send + Sync {
    async fn get_conversation(&self, session_id: &Uuid) -> Result<Vec<Message>, MemoryError>;
    async fn append_message(&self, session_id: &Uuid, msg: Message) -> Result<(), MemoryError>;
    async fn summarize(&self, session_id: &Uuid) -> Result<String, MemoryError>;
    async fn clear(&self, session_id: &Uuid) -> Result<(), MemoryError>;
}

/// Long-term memory (vector DB based RAG)
#[async_trait]
pub trait LongTermMemory: Send + Sync {
    async fn store(&self, doc: MemoryDocument) -> Result<String, MemoryError>;
    async fn search(&self, query: &str, top_k: usize) -> Result<Vec<SearchResult>, MemoryError>;
    async fn delete(&self, doc_id: &str) -> Result<(), MemoryError>;
}

/// Episodic memory (Agent decision history)
#[async_trait]
pub trait EpisodicMemory: Send + Sync {
    async fn record_episode(&self, episode: Episode) -> Result<(), MemoryError>;
    async fn recall(
        &self,
        agent_id: &str,
        context: &str,
        limit: usize,
    ) -> Result<Vec<Episode>, MemoryError>;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryDocument {
    pub id: String,
    pub content: String,
    pub source: String,
    pub agent_id: String,
    pub metadata: HashMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchResult {
    pub id: String,
    pub score: f32,
    pub content: String,
    pub source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Episode {
    pub agent_id: String,
    pub session_id: Uuid,
    pub action_taken: String,
    pub reasoning: String,
    pub outcome: String,
    pub timestamp: DateTime<Utc>,
}
