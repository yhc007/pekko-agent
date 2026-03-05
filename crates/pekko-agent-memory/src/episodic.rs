use async_trait::async_trait;
use pekko_agent_core::{EpisodicMemory, Episode, MemoryError};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, debug};

/// In-memory episodic store for agent action history
/// 
/// Production deployments would use PostgreSQL, MongoDB, or similar persistent database
pub struct InMemoryEpisodicStore {
    episodes: Arc<RwLock<HashMap<String, Vec<Episode>>>>,
}

impl InMemoryEpisodicStore {
    /// Create a new in-memory episodic store
    pub fn new() -> Self {
        Self {
            episodes: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get the number of agents with recorded episodes
    pub async fn agent_count(&self) -> usize {
        self.episodes.read().await.len()
    }

    /// Get the number of episodes for a specific agent
    pub async fn episode_count(&self, agent_id: &str) -> usize {
        self.episodes
            .read()
            .await
            .get(agent_id)
            .map(|eps| eps.len())
            .unwrap_or(0)
    }

    /// Get all episodes for an agent (paginated)
    pub async fn get_all_episodes(
        &self,
        agent_id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<Episode>, MemoryError> {
        let store = self.episodes.read().await;
        let episodes = store
            .get(agent_id)
            .ok_or_else(|| MemoryError::NotFound(format!("No episodes for agent {}", agent_id)))?;

        let start = offset.min(episodes.len());
        let end = (offset + limit).min(episodes.len());

        Ok(episodes[start..end].to_vec())
    }

    /// Delete all episodes for an agent
    pub async fn delete_agent_episodes(&self, agent_id: &str) -> Result<(), MemoryError> {
        let mut store = self.episodes.write().await;
        store.remove(agent_id);
        info!(agent_id = %agent_id, "Deleted all episodes for agent");
        Ok(())
    }
}

#[async_trait]
impl EpisodicMemory for InMemoryEpisodicStore {
    async fn record_episode(&self, episode: Episode) -> Result<(), MemoryError> {
        let mut store = self.episodes.write().await;
        let agent_episodes = store
            .entry(episode.agent_id.clone())
            .or_insert_with(Vec::new);

        info!(
            agent_id = %episode.agent_id,
            action = %episode.action_taken,
            outcome = %episode.outcome,
            "Recording episode"
        );

        agent_episodes.push(episode);
        Ok(())
    }

    async fn recall(
        &self,
        agent_id: &str,
        context: &str,
        limit: usize,
    ) -> Result<Vec<Episode>, MemoryError> {
        let store = self.episodes.read().await;

        let episodes = store
            .get(agent_id)
            .ok_or_else(|| MemoryError::NotFound(format!("No episodes for agent {}", agent_id)))?;

        debug!(
            agent_id = %agent_id,
            context = %context,
            total_episodes = episodes.len(),
            "Recalling episodes with context"
        );

        // Filter episodes that are relevant to the context
        let context_lower = context.to_lowercase();
        let mut relevant_episodes: Vec<_> = episodes
            .iter()
            .filter(|ep| {
                let action_match = ep.action_taken.to_lowercase().contains(&context_lower);
                let outcome_match = ep.outcome.to_lowercase().contains(&context_lower);
                action_match || outcome_match
            })
            .cloned()
            .collect();

        // Sort by timestamp, most recent first
        relevant_episodes.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        // Return only the requested limit
        relevant_episodes.truncate(limit);

        debug!(
            agent_id = %agent_id,
            matched = relevant_episodes.len(),
            "Recall completed"
        );

        Ok(relevant_episodes)
    }
}

impl Clone for InMemoryEpisodicStore {
    fn clone(&self) -> Self {
        Self {
            episodes: Arc::clone(&self.episodes),
        }
    }
}

impl Default for InMemoryEpisodicStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_episodic_store_creation() {
        let store = InMemoryEpisodicStore::new();
        assert_eq!(store.agent_count().await, 0);
    }

    #[tokio::test]
    async fn test_record_episode() {
        let store = InMemoryEpisodicStore::new();
        let episode = Episode {
            agent_id: "agent-1".to_string(),
            action_taken: "execute_permit_search".to_string(),
            outcome: "found 3 permits".to_string(),
            timestamp: chrono::Utc::now(),
            context: serde_json::json!({"query": "environmental"}),
        };

        assert!(store.record_episode(episode).await.is_ok());
        assert_eq!(store.agent_count().await, 1);
        assert_eq!(store.episode_count("agent-1").await, 1);
    }

    #[tokio::test]
    async fn test_record_multiple_episodes() {
        let store = InMemoryEpisodicStore::new();

        for i in 0..5 {
            let episode = Episode {
                agent_id: "agent-1".to_string(),
                action_taken: format!("action-{}", i),
                outcome: format!("outcome-{}", i),
                timestamp: chrono::Utc::now(),
                context: serde_json::json!({}),
            };
            let _ = store.record_episode(episode).await;
        }

        assert_eq!(store.episode_count("agent-1").await, 5);
    }

    #[tokio::test]
    async fn test_recall_episodes() {
        let store = InMemoryEpisodicStore::new();

        let episode1 = Episode {
            agent_id: "agent-1".to_string(),
            action_taken: "permit_search".to_string(),
            outcome: "found permits".to_string(),
            timestamp: chrono::Utc::now(),
            context: serde_json::json!({}),
        };

        let episode2 = Episode {
            agent_id: "agent-1".to_string(),
            action_taken: "compliance_check".to_string(),
            outcome: "checked compliance".to_string(),
            timestamp: chrono::Utc::now(),
            context: serde_json::json!({}),
        };

        let _ = store.record_episode(episode1).await;
        let _ = store.record_episode(episode2).await;

        let recalled = store.recall("agent-1", "permit", 10).await.unwrap();
        assert_eq!(recalled.len(), 1);
        assert!(recalled[0].action_taken.contains("permit"));
    }

    #[tokio::test]
    async fn test_recall_limit() {
        let store = InMemoryEpisodicStore::new();

        for i in 0..10 {
            let episode = Episode {
                agent_id: "agent-1".to_string(),
                action_taken: "permit_search".to_string(),
                outcome: format!("outcome-{}", i),
                timestamp: chrono::Utc::now(),
                context: serde_json::json!({}),
            };
            let _ = store.record_episode(episode).await;
        }

        let recalled = store.recall("agent-1", "permit", 3).await.unwrap();
        assert_eq!(recalled.len(), 3);
    }

    #[tokio::test]
    async fn test_recall_no_episodes() {
        let store = InMemoryEpisodicStore::new();
        let result = store.recall("agent-1", "anything", 10).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_recall_context_filtering() {
        let store = InMemoryEpisodicStore::new();

        let episode1 = Episode {
            agent_id: "agent-1".to_string(),
            action_taken: "PERMIT_SEARCH".to_string(),
            outcome: "success".to_string(),
            timestamp: chrono::Utc::now(),
            context: serde_json::json!({}),
        };

        let episode2 = Episode {
            agent_id: "agent-1".to_string(),
            action_taken: "compliance_check".to_string(),
            outcome: "success".to_string(),
            timestamp: chrono::Utc::now(),
            context: serde_json::json!({}),
        };

        let _ = store.record_episode(episode1).await;
        let _ = store.record_episode(episode2).await;

        // Case-insensitive search
        let recalled = store.recall("agent-1", "permit", 10).await.unwrap();
        assert_eq!(recalled.len(), 1);
    }

    #[tokio::test]
    async fn test_get_all_episodes() {
        let store = InMemoryEpisodicStore::new();

        for i in 0..5 {
            let episode = Episode {
                agent_id: "agent-1".to_string(),
                action_taken: format!("action-{}", i),
                outcome: format!("outcome-{}", i),
                timestamp: chrono::Utc::now(),
                context: serde_json::json!({}),
            };
            let _ = store.record_episode(episode).await;
        }

        let episodes = store.get_all_episodes("agent-1", 0, 10).await.unwrap();
        assert_eq!(episodes.len(), 5);
    }

    #[tokio::test]
    async fn test_get_all_episodes_pagination() {
        let store = InMemoryEpisodicStore::new();

        for i in 0..10 {
            let episode = Episode {
                agent_id: "agent-1".to_string(),
                action_taken: format!("action-{}", i),
                outcome: format!("outcome-{}", i),
                timestamp: chrono::Utc::now(),
                context: serde_json::json!({}),
            };
            let _ = store.record_episode(episode).await;
        }

        let page1 = store.get_all_episodes("agent-1", 0, 3).await.unwrap();
        let page2 = store.get_all_episodes("agent-1", 3, 3).await.unwrap();

        assert_eq!(page1.len(), 3);
        assert_eq!(page2.len(), 3);
    }

    #[tokio::test]
    async fn test_delete_agent_episodes() {
        let store = InMemoryEpisodicStore::new();
        let episode = Episode {
            agent_id: "agent-1".to_string(),
            action_taken: "test".to_string(),
            outcome: "test".to_string(),
            timestamp: chrono::Utc::now(),
            context: serde_json::json!({}),
        };

        let _ = store.record_episode(episode).await;
        assert_eq!(store.agent_count().await, 1);

        let _ = store.delete_agent_episodes("agent-1").await;
        assert_eq!(store.agent_count().await, 0);
    }
}
