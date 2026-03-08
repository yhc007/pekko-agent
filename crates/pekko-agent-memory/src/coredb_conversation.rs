use async_trait::async_trait;
use pekko_agent_core::{ShortTermMemory, Message, MessageRole, MemoryError};
use coredb::{CoreDB, DatabaseConfig, QueryResult};
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;
use tracing::{info, debug, warn};

/// CoreDB-backed conversation store
///
/// Uses embedded coreDB (Cassandra-style NoSQL) for persistent conversation storage.
/// Replaces Redis with a single embedded database — no external dependencies needed.
pub struct CoreDbConversationStore {
    db: Arc<CoreDB>,
    max_messages: usize,
    keyspace: String,
}

impl CoreDbConversationStore {
    /// Create a new CoreDB-backed conversation store
    ///
    /// # Arguments
    /// * `db` - Shared CoreDB instance
    /// * `max_messages` - Maximum messages to keep per conversation
    pub async fn new(db: Arc<CoreDB>, max_messages: usize) -> Result<Self, MemoryError> {
        let keyspace = "pekko_agent".to_string();

        // Initialize schema
        Self::init_schema(&db, &keyspace).await?;

        info!(keyspace = %keyspace, "CoreDB conversation store initialized");

        Ok(Self {
            db,
            max_messages,
            keyspace,
        })
    }

    /// Create with a new embedded CoreDB instance
    pub async fn with_embedded(
        data_dir: PathBuf,
        max_messages: usize,
    ) -> Result<Self, MemoryError> {
        let config = DatabaseConfig {
            data_directory: data_dir.join("data"),
            commitlog_directory: data_dir.join("commitlog"),
            ..Default::default()
        };

        let db = CoreDB::new(config).await.map_err(|e| {
            MemoryError::StorageError(format!("Failed to initialize CoreDB: {}", e))
        })?;

        Self::new(Arc::new(db), max_messages).await
    }

    /// Initialize keyspace and table schema
    async fn init_schema(db: &CoreDB, keyspace: &str) -> Result<(), MemoryError> {
        // Create keyspace
        let cql = format!(
            "CREATE KEYSPACE IF NOT EXISTS {} WITH REPLICATION = {{'class': 'SimpleStrategy', 'replication_factor': 1}}",
            keyspace
        );
        db.execute_cql(&cql).await.map_err(|e| {
            MemoryError::StorageError(format!("Failed to create keyspace: {}", e))
        })?;

        // Create conversations table
        // partition key: session_id, clustering key: seq_num (for ordering)
        let cql = format!(
            "CREATE TABLE IF NOT EXISTS {}.conversations (
                session_id TEXT,
                seq_num BIGINT,
                role TEXT,
                content TEXT,
                timestamp TEXT,
                PRIMARY KEY (session_id, seq_num)
            )",
            keyspace
        );
        db.execute_cql(&cql).await.map_err(|e| {
            MemoryError::StorageError(format!("Failed to create conversations table: {}", e))
        })?;

        debug!("CoreDB schema initialized for conversations");
        Ok(())
    }

    /// Get current max sequence number for a session
    async fn get_max_seq(&self, session_id: &Uuid) -> Result<i64, MemoryError> {
        let cql = format!(
            "SELECT MAX(seq_num) FROM {}.conversations WHERE session_id = '{}'",
            self.keyspace, session_id
        );

        match self.db.execute_cql(&cql).await {
            Ok(QueryResult::Rows(rows)) => {
                if let Some(row) = rows.first() {
                    if let Some(coredb::CassandraValue::BigInt(n)) = row.columns.get("max_seq_num") {
                        return Ok(*n);
                    }
                    // Try system_max(seq_num) column name variant
                    for (_, val) in &row.columns {
                        if let coredb::CassandraValue::BigInt(n) = val {
                            return Ok(*n);
                        }
                    }
                }
                Ok(0)
            }
            Ok(_) => Ok(0),
            Err(_) => Ok(0),
        }
    }

    /// Get the number of active conversations
    pub async fn conversation_count(&self) -> usize {
        let cql = format!(
            "SELECT DISTINCT session_id FROM {}.conversations",
            self.keyspace
        );
        match self.db.execute_cql(&cql).await {
            Ok(QueryResult::Rows(rows)) => rows.len(),
            _ => 0,
        }
    }

    /// Get the number of messages in a conversation
    pub async fn message_count(&self, session_id: &Uuid) -> usize {
        let cql = format!(
            "SELECT COUNT(*) FROM {}.conversations WHERE session_id = '{}'",
            self.keyspace, session_id
        );
        match self.db.execute_cql(&cql).await {
            Ok(QueryResult::Rows(rows)) => {
                if let Some(row) = rows.first() {
                    for (_, val) in &row.columns {
                        if let coredb::CassandraValue::BigInt(n) = val {
                            return *n as usize;
                        }
                        if let coredb::CassandraValue::Int(n) = val {
                            return *n as usize;
                        }
                    }
                }
                0
            }
            _ => 0,
        }
    }

    /// Delete a conversation
    pub async fn delete_conversation(&self, session_id: &Uuid) -> Result<(), MemoryError> {
        let cql = format!(
            "DELETE FROM {}.conversations WHERE session_id = '{}'",
            self.keyspace, session_id
        );
        self.db.execute_cql(&cql).await.map_err(|e| {
            MemoryError::StorageError(format!("Failed to delete conversation: {}", e))
        })?;
        info!(session_id = %session_id, "Conversation deleted from CoreDB");
        Ok(())
    }

    /// Convert CQL row to Message
    fn row_to_message(row: &coredb::query::result::Row) -> Option<Message> {
        let role_str = match row.columns.get("role") {
            Some(coredb::CassandraValue::Text(s)) => s.clone(),
            _ => return None,
        };
        let content = match row.columns.get("content") {
            Some(coredb::CassandraValue::Text(s)) => s.clone(),
            _ => return None,
        };
        let timestamp_str = match row.columns.get("timestamp") {
            Some(coredb::CassandraValue::Text(s)) => s.clone(),
            _ => return None,
        };

        let role = match role_str.as_str() {
            "user" => MessageRole::User,
            "assistant" => MessageRole::Assistant,
            "system" => MessageRole::System,
            "tool" => MessageRole::Tool,
            _ => MessageRole::User,
        };

        let timestamp = chrono::DateTime::parse_from_rfc3339(&timestamp_str)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now());

        Some(Message {
            role,
            content,
            timestamp,
        })
    }
}

#[async_trait]
impl ShortTermMemory for CoreDbConversationStore {
    async fn get_conversation(&self, session_id: &Uuid) -> Result<Vec<Message>, MemoryError> {
        let cql = format!(
            "SELECT role, content, timestamp FROM {}.conversations WHERE session_id = '{}' ORDER BY seq_num",
            self.keyspace, session_id
        );

        match self.db.execute_cql(&cql).await {
            Ok(QueryResult::Rows(rows)) => {
                let messages: Vec<Message> = rows
                    .iter()
                    .filter_map(|row| Self::row_to_message(row))
                    .collect();
                Ok(messages)
            }
            Ok(_) => Ok(vec![]),
            Err(e) => {
                warn!(error = %e, "Failed to get conversation from CoreDB");
                Ok(vec![])
            }
        }
    }

    async fn append_message(&self, session_id: &Uuid, msg: Message) -> Result<(), MemoryError> {
        let seq = self.get_max_seq(session_id).await? + 1;

        let role_str = match msg.role {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::System => "system",
            MessageRole::Tool => "tool",
        };

        let timestamp_str = msg.timestamp.to_rfc3339();
        // Escape single quotes in content
        let escaped_content = msg.content.replace('\'', "''");

        let cql = format!(
            "INSERT INTO {}.conversations (session_id, seq_num, role, content, timestamp) VALUES ('{}', {}, '{}', '{}', '{}')",
            self.keyspace, session_id, seq, role_str, escaped_content, timestamp_str
        );

        self.db.execute_cql(&cql).await.map_err(|e| {
            MemoryError::StorageError(format!("Failed to append message: {}", e))
        })?;

        debug!(session_id = %session_id, role = %role_str, seq = seq, "Message appended to CoreDB");

        // Enforce max_messages limit — delete oldest messages if over limit
        let count = self.message_count(session_id).await;
        if count > self.max_messages {
            let overflow = count - self.max_messages;
            // Get the seq_nums of oldest messages to delete
            let cql = format!(
                "SELECT seq_num FROM {}.conversations WHERE session_id = '{}' ORDER BY seq_num LIMIT {}",
                self.keyspace, session_id, overflow
            );
            if let Ok(QueryResult::Rows(rows)) = self.db.execute_cql(&cql).await {
                for row in &rows {
                    if let Some(coredb::CassandraValue::BigInt(seq_to_delete)) = row.columns.get("seq_num") {
                        let delete_cql = format!(
                            "DELETE FROM {}.conversations WHERE session_id = '{}' AND seq_num = {}",
                            self.keyspace, session_id, seq_to_delete
                        );
                        let _ = self.db.execute_cql(&delete_cql).await;
                    }
                }
                debug!(session_id = %session_id, removed = overflow, "Trimmed conversation history in CoreDB");
            }
        }

        Ok(())
    }

    async fn summarize(&self, session_id: &Uuid) -> Result<String, MemoryError> {
        let messages = self.get_conversation(session_id).await?;

        if messages.is_empty() {
            return Err(MemoryError::NotFound(format!("Session {} not found", session_id)));
        }

        let summary = messages
            .iter()
            .map(|m| format!("{}: {}", m.role, m.content))
            .collect::<Vec<_>>()
            .join("\n---\n");

        Ok(summary)
    }

    async fn clear(&self, session_id: &Uuid) -> Result<(), MemoryError> {
        self.delete_conversation(session_id).await?;
        info!(session_id = %session_id, "Conversation cleared in CoreDB");
        Ok(())
    }
}

impl Clone for CoreDbConversationStore {
    fn clone(&self) -> Self {
        Self {
            db: Arc::clone(&self.db),
            max_messages: self.max_messages,
            keyspace: self.keyspace.clone(),
        }
    }
}
