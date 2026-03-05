use async_trait::async_trait;
use pekko_agent_core::{ShortTermMemory, Message, MemoryError};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;
use tracing::{info, debug};

/// In-memory conversation store
/// 
/// Production deployments would use Redis or similar distributed cache
pub struct InMemoryConversationStore {
    conversations: Arc<RwLock<HashMap<Uuid, Vec<Message>>>>,
    max_messages: usize,
}

impl InMemoryConversationStore {
    /// Create a new in-memory conversation store
    ///
    /// # Arguments
    /// * `max_messages` - Maximum messages to keep per conversation (older messages are discarded)
    pub fn new(max_messages: usize) -> Self {
        Self {
            conversations: Arc::new(RwLock::new(HashMap::new())),
            max_messages,
        }
    }

    /// Get the number of active conversations
    pub async fn conversation_count(&self) -> usize {
        self.conversations.read().await.len()
    }

    /// Get the number of messages in a conversation
    pub async fn message_count(&self, session_id: &Uuid) -> usize {
        self.conversations
            .read()
            .await
            .get(session_id)
            .map(|msgs| msgs.len())
            .unwrap_or(0)
    }

    /// Delete a conversation
    pub async fn delete_conversation(&self, session_id: &Uuid) -> Result<(), MemoryError> {
        let mut store = self.conversations.write().await;
        store.remove(session_id);
        info!(session_id = %session_id, "Conversation deleted");
        Ok(())
    }
}

#[async_trait]
impl ShortTermMemory for InMemoryConversationStore {
    async fn get_conversation(&self, session_id: &Uuid) -> Result<Vec<Message>, MemoryError> {
        let store = self.conversations.read().await;
        Ok(store.get(session_id).cloned().unwrap_or_default())
    }

    async fn append_message(&self, session_id: &Uuid, msg: Message) -> Result<(), MemoryError> {
        let mut store = self.conversations.write().await;
        let messages = store.entry(*session_id).or_insert_with(Vec::new);
        
        debug!(
            session_id = %session_id,
            role = %msg.role,
            "Appending message to conversation"
        );
        
        messages.push(msg);
        
        // Enforce max_messages limit by removing oldest messages
        if messages.len() > self.max_messages {
            let drain_count = messages.len() - self.max_messages;
            messages.drain(..drain_count);
            debug!(
                session_id = %session_id,
                removed = drain_count,
                "Trimmed conversation history"
            );
        }
        
        Ok(())
    }

    async fn summarize(&self, session_id: &Uuid) -> Result<String, MemoryError> {
        let store = self.conversations.read().await;
        let messages = store.get(session_id).ok_or_else(|| {
            MemoryError::NotFound(format!("Session {} not found", session_id))
        })?;
        
        // Build a summary by concatenating message contents
        let summary = messages
            .iter()
            .map(|m| format!("{}: {}", m.role, m.content))
            .collect::<Vec<_>>()
            .join("\n---\n");
        
        Ok(summary)
    }

    async fn clear(&self, session_id: &Uuid) -> Result<(), MemoryError> {
        let mut store = self.conversations.write().await;
        store.remove(session_id);
        info!(session_id = %session_id, "Conversation cleared");
        Ok(())
    }
}

impl Clone for InMemoryConversationStore {
    fn clone(&self) -> Self {
        Self {
            conversations: Arc::clone(&self.conversations),
            max_messages: self.max_messages,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pekko_agent_core::MessageRole;

    fn test_msg(content: &str) -> Message {
        Message { role: MessageRole::User, content: content.to_string(), timestamp: chrono::Utc::now() }
    }

    #[tokio::test]
    async fn test_conversation_store_creation() {
        let store = InMemoryConversationStore::new(100);
        assert_eq!(store.conversation_count().await, 0);
    }

    #[tokio::test]
    async fn test_append_message() {
        let store = InMemoryConversationStore::new(100);
        let session_id = Uuid::new_v4();
        assert!(store.append_message(&session_id, test_msg("Hello")).await.is_ok());
        assert_eq!(store.message_count(&session_id).await, 1);
    }

    #[tokio::test]
    async fn test_get_conversation() {
        let store = InMemoryConversationStore::new(100);
        let session_id = Uuid::new_v4();
        let _ = store.append_message(&session_id, test_msg("Hello")).await;
        let messages = store.get_conversation(&session_id).await.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "Hello");
    }

    #[tokio::test]
    async fn test_max_messages_limit() {
        let max = 5;
        let store = InMemoryConversationStore::new(max);
        let session_id = Uuid::new_v4();
        for i in 0..10 {
            let _ = store.append_message(&session_id, test_msg(&format!("Message {}", i))).await;
        }
        let messages = store.get_conversation(&session_id).await.unwrap();
        assert_eq!(messages.len(), max);
        assert_eq!(messages[0].content, "Message 5");
    }

    #[tokio::test]
    async fn test_clear_conversation() {
        let store = InMemoryConversationStore::new(100);
        let session_id = Uuid::new_v4();
        let _ = store.append_message(&session_id, test_msg("Hello")).await;
        assert_eq!(store.message_count(&session_id).await, 1);
        let _ = store.clear(&session_id).await;
        assert_eq!(store.message_count(&session_id).await, 0);
    }

    #[tokio::test]
    async fn test_summarize() {
        let store = InMemoryConversationStore::new(100);
        let session_id = Uuid::new_v4();
        let _ = store.append_message(&session_id, test_msg("Hello")).await;
        let _ = store.append_message(&session_id, Message {
            role: MessageRole::Assistant, content: "Hi there".to_string(), timestamp: chrono::Utc::now(),
        }).await;
        let summary = store.summarize(&session_id).await.unwrap();
        assert!(summary.contains("user"));
        assert!(summary.contains("assistant"));
    }

    #[tokio::test]
    async fn test_summarize_nonexistent() {
        let store = InMemoryConversationStore::new(100);
        let result = store.summarize(&Uuid::new_v4()).await;
        assert!(result.is_err());
    }
}
