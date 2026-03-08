use async_trait::async_trait;
use pekko_agent_core::{EpisodicMemory, Episode, MemoryError};
use coredb::{CoreDB, QueryResult, CassandraValue};
use std::sync::Arc;
use uuid::Uuid;
use tracing::{info, debug, warn};

/// CoreDB-backed episodic store for agent action history
///
/// Uses embedded coreDB for persistent episodic memory.
/// Replaces PostgreSQL with a single embedded database.
pub struct CoreDbEpisodicStore {
    db: Arc<CoreDB>,
    keyspace: String,
}

impl CoreDbEpisodicStore {
    /// Create a new CoreDB-backed episodic store
    pub async fn new(db: Arc<CoreDB>) -> Result<Self, MemoryError> {
        let keyspace = "pekko_agent".to_string();

        Self::init_schema(&db, &keyspace).await?;

        info!(keyspace = %keyspace, "CoreDB episodic store initialized");

        Ok(Self { db, keyspace })
    }

    /// Initialize table schema for episodes
    async fn init_schema(db: &CoreDB, keyspace: &str) -> Result<(), MemoryError> {
        // Create keyspace (idempotent)
        let cql = format!(
            "CREATE KEYSPACE IF NOT EXISTS {} WITH REPLICATION = {{'class': 'SimpleStrategy', 'replication_factor': 1}}",
            keyspace
        );
        db.execute_cql(&cql).await.map_err(|e| {
            MemoryError::StorageError(format!("Failed to create keyspace: {}", e))
        })?;

        // Create episodes table
        // partition key: agent_id, clustering key: timestamp (DESC for recent-first recall)
        let cql = format!(
            "CREATE TABLE IF NOT EXISTS {}.episodes (
                agent_id TEXT,
                timestamp TEXT,
                session_id TEXT,
                action_taken TEXT,
                reasoning TEXT,
                outcome TEXT,
                PRIMARY KEY (agent_id, timestamp)
            )",
            keyspace
        );
        db.execute_cql(&cql).await.map_err(|e| {
            MemoryError::StorageError(format!("Failed to create episodes table: {}", e))
        })?;

        debug!("CoreDB schema initialized for episodes");
        Ok(())
    }

    /// Get the number of agents with recorded episodes
    pub async fn agent_count(&self) -> usize {
        let cql = format!(
            "SELECT DISTINCT agent_id FROM {}.episodes",
            self.keyspace
        );
        match self.db.execute_cql(&cql).await {
            Ok(QueryResult::Rows(rows)) => rows.len(),
            _ => 0,
        }
    }

    /// Get the number of episodes for a specific agent
    pub async fn episode_count(&self, agent_id: &str) -> usize {
        let cql = format!(
            "SELECT COUNT(*) FROM {}.episodes WHERE agent_id = '{}'",
            self.keyspace, agent_id
        );
        match self.db.execute_cql(&cql).await {
            Ok(QueryResult::Rows(rows)) => {
                if let Some(row) = rows.first() {
                    for (_, val) in &row.columns {
                        if let CassandraValue::BigInt(n) = val {
                            return *n as usize;
                        }
                        if let CassandraValue::Int(n) = val {
                            return *n as usize;
                        }
                    }
                }
                0
            }
            _ => 0,
        }
    }

    /// Get all episodes for an agent (paginated)
    pub async fn get_all_episodes(
        &self,
        agent_id: &str,
        _offset: usize,
        limit: usize,
    ) -> Result<Vec<Episode>, MemoryError> {
        let cql = format!(
            "SELECT agent_id, session_id, action_taken, reasoning, outcome, timestamp FROM {}.episodes WHERE agent_id = '{}' LIMIT {}",
            self.keyspace, agent_id, limit
        );

        match self.db.execute_cql(&cql).await {
            Ok(QueryResult::Rows(rows)) => {
                let episodes: Vec<Episode> = rows
                    .iter()
                    .filter_map(|row| Self::row_to_episode(row))
                    .collect();
                Ok(episodes)
            }
            Ok(_) => Err(MemoryError::NotFound(format!("No episodes for agent {}", agent_id))),
            Err(e) => Err(MemoryError::StorageError(format!("Failed to query episodes: {}", e))),
        }
    }

    /// Delete all episodes for an agent
    pub async fn delete_agent_episodes(&self, agent_id: &str) -> Result<(), MemoryError> {
        let cql = format!(
            "DELETE FROM {}.episodes WHERE agent_id = '{}'",
            self.keyspace, agent_id
        );
        self.db.execute_cql(&cql).await.map_err(|e| {
            MemoryError::StorageError(format!("Failed to delete episodes: {}", e))
        })?;
        info!(agent_id = %agent_id, "Deleted all episodes from CoreDB");
        Ok(())
    }

    /// Convert CQL result row to Episode
    fn row_to_episode(row: &coredb::query::result::Row) -> Option<Episode> {
        let agent_id = match row.columns.get("agent_id") {
            Some(CassandraValue::Text(s)) => s.clone(),
            _ => return None,
        };
        let session_id_str = match row.columns.get("session_id") {
            Some(CassandraValue::Text(s)) => s.clone(),
            _ => Uuid::new_v4().to_string(),
        };
        let action_taken = match row.columns.get("action_taken") {
            Some(CassandraValue::Text(s)) => s.clone(),
            _ => return None,
        };
        let reasoning = match row.columns.get("reasoning") {
            Some(CassandraValue::Text(s)) => s.clone(),
            _ => String::new(),
        };
        let outcome = match row.columns.get("outcome") {
            Some(CassandraValue::Text(s)) => s.clone(),
            _ => return None,
        };
        let timestamp_str = match row.columns.get("timestamp") {
            Some(CassandraValue::Text(s)) => s.clone(),
            _ => return None,
        };

        let session_id = Uuid::parse_str(&session_id_str).unwrap_or_else(|_| Uuid::new_v4());
        let timestamp = chrono::DateTime::parse_from_rfc3339(&timestamp_str)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now());

        Some(Episode {
            agent_id,
            session_id,
            action_taken,
            reasoning,
            outcome,
            timestamp,
        })
    }
}

#[async_trait]
impl EpisodicMemory for CoreDbEpisodicStore {
    async fn record_episode(&self, episode: Episode) -> Result<(), MemoryError> {
        let escaped_action = episode.action_taken.replace('\'', "''");
        let escaped_reasoning = episode.reasoning.replace('\'', "''");
        let escaped_outcome = episode.outcome.replace('\'', "''");
        let timestamp_str = episode.timestamp.to_rfc3339();

        let cql = format!(
            "INSERT INTO {}.episodes (agent_id, timestamp, session_id, action_taken, reasoning, outcome) VALUES ('{}', '{}', '{}', '{}', '{}', '{}')",
            self.keyspace,
            episode.agent_id,
            timestamp_str,
            episode.session_id,
            escaped_action,
            escaped_reasoning,
            escaped_outcome
        );

        self.db.execute_cql(&cql).await.map_err(|e| {
            MemoryError::StorageError(format!("Failed to record episode: {}", e))
        })?;

        info!(
            agent_id = %episode.agent_id,
            action = %episode.action_taken,
            outcome = %episode.outcome,
            "Episode recorded in CoreDB"
        );

        Ok(())
    }

    async fn recall(
        &self,
        agent_id: &str,
        context: &str,
        limit: usize,
    ) -> Result<Vec<Episode>, MemoryError> {
        // Fetch all episodes for the agent (CoreDB doesn't support LIKE or full-text search natively)
        let cql = format!(
            "SELECT agent_id, session_id, action_taken, reasoning, outcome, timestamp FROM {}.episodes WHERE agent_id = '{}'",
            self.keyspace, agent_id
        );

        match self.db.execute_cql(&cql).await {
            Ok(QueryResult::Rows(rows)) => {
                let context_lower = context.to_lowercase();

                let mut relevant: Vec<Episode> = rows
                    .iter()
                    .filter_map(|row| Self::row_to_episode(row))
                    .filter(|ep| {
                        ep.action_taken.to_lowercase().contains(&context_lower)
                            || ep.outcome.to_lowercase().contains(&context_lower)
                            || ep.reasoning.to_lowercase().contains(&context_lower)
                    })
                    .collect();

                // Sort by timestamp descending (most recent first)
                relevant.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
                relevant.truncate(limit);

                debug!(
                    agent_id = %agent_id,
                    context = %context,
                    matched = relevant.len(),
                    "Recall completed from CoreDB"
                );

                Ok(relevant)
            }
            Ok(_) => Err(MemoryError::NotFound(format!("No episodes for agent {}", agent_id))),
            Err(e) => Err(MemoryError::StorageError(format!("Failed to recall episodes: {}", e))),
        }
    }
}

impl Clone for CoreDbEpisodicStore {
    fn clone(&self) -> Self {
        Self {
            db: Arc::clone(&self.db),
            keyspace: self.keyspace.clone(),
        }
    }
}

impl Default for CoreDbEpisodicStore {
    fn default() -> Self {
        // This will panic if called without proper async initialization
        // Use CoreDbEpisodicStore::new() instead
        panic!("Use CoreDbEpisodicStore::new(db) for proper initialization")
    }
}
