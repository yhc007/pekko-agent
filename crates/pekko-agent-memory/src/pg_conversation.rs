use async_trait::async_trait;
use pekko_agent_core::{Message, MessageRole, MemoryError, ShortTermMemory};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;
use tracing::{debug, info};

/// PostgreSQL-backed conversation store.
///
/// Schema (auto-created on first use):
///
/// ```sql
/// CREATE TABLE pekko_sessions (
///     session_id  UUID        NOT NULL,
///     seq_num     BIGSERIAL,
///     role        TEXT        NOT NULL,
///     content     TEXT        NOT NULL,
///     created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
///     PRIMARY KEY (session_id, seq_num)
/// );
/// ```
pub struct PgConversationStore {
    pool: Arc<PgPool>,
    max_messages: usize,
}

impl PgConversationStore {
    pub fn new(pool: Arc<PgPool>, max_messages: usize) -> Self {
        Self { pool, max_messages }
    }

    /// Create the table if it does not exist yet.
    pub async fn migrate(pool: &PgPool) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS pekko_sessions (
                session_id  UUID        NOT NULL,
                seq_num     BIGSERIAL,
                role        TEXT        NOT NULL,
                content     TEXT        NOT NULL,
                created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                PRIMARY KEY (session_id, seq_num)
            )
            "#,
        )
        .execute(pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS pekko_sessions_session_id_idx ON pekko_sessions (session_id)",
        )
        .execute(pool)
        .await?;

        info!("pekko_sessions table ready");
        Ok(())
    }

    /// Trim the session to at most `max_messages` rows, deleting the oldest.
    async fn trim(&self, session_id: &Uuid) -> Result<(), MemoryError> {
        let max = self.max_messages as i64;
        sqlx::query(
            r#"
            DELETE FROM pekko_sessions
            WHERE session_id = $1
              AND seq_num NOT IN (
                  SELECT seq_num FROM pekko_sessions
                  WHERE session_id = $1
                  ORDER BY seq_num DESC
                  LIMIT $2
              )
            "#,
        )
        .bind(session_id)
        .bind(max)
        .execute(self.pool.as_ref())
        .await
        .map_err(|e| MemoryError::StorageError(e.to_string()))?;
        Ok(())
    }
}

#[async_trait]
impl ShortTermMemory for PgConversationStore {
    async fn get_conversation(&self, session_id: &Uuid) -> Result<Vec<Message>, MemoryError> {
        let rows = sqlx::query_as::<_, (String, String, chrono::DateTime<chrono::Utc>)>(
            "SELECT role, content, created_at FROM pekko_sessions \
             WHERE session_id = $1 ORDER BY seq_num ASC",
        )
        .bind(session_id)
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(|e| MemoryError::StorageError(e.to_string()))?;

        debug!(session_id = %session_id, count = rows.len(), "Loaded conversation");

        let messages = rows
            .into_iter()
            .map(|(role, content, timestamp)| Message {
                role: match role.as_str() {
                    "assistant" => MessageRole::Assistant,
                    "system" => MessageRole::System,
                    _ => MessageRole::User,
                },
                content,
                timestamp,
            })
            .collect();

        Ok(messages)
    }

    async fn append_message(&self, session_id: &Uuid, msg: Message) -> Result<(), MemoryError> {
        let role = match msg.role {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::System => "system",
            _ => "user",
        };

        sqlx::query(
            "INSERT INTO pekko_sessions (session_id, role, content, created_at) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(session_id)
        .bind(role)
        .bind(&msg.content)
        .bind(msg.timestamp)
        .execute(self.pool.as_ref())
        .await
        .map_err(|e| MemoryError::StorageError(e.to_string()))?;

        self.trim(session_id).await?;

        debug!(session_id = %session_id, role = role, "Message appended");
        Ok(())
    }

    async fn summarize(&self, session_id: &Uuid) -> Result<String, MemoryError> {
        let messages = self.get_conversation(session_id).await?;
        if messages.is_empty() {
            return Err(MemoryError::NotFound(format!(
                "Session {} not found",
                session_id
            )));
        }
        let summary = messages
            .iter()
            .map(|m| format!("{}: {}", m.role, m.content))
            .collect::<Vec<_>>()
            .join("\n---\n");
        Ok(summary)
    }

    async fn clear(&self, session_id: &Uuid) -> Result<(), MemoryError> {
        sqlx::query("DELETE FROM pekko_sessions WHERE session_id = $1")
            .bind(session_id)
            .execute(self.pool.as_ref())
            .await
            .map_err(|e| MemoryError::StorageError(e.to_string()))?;

        info!(session_id = %session_id, "Conversation cleared");
        Ok(())
    }
}
