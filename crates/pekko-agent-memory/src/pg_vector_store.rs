use async_trait::async_trait;
use pekko_agent_core::{Embedder, LongTermMemory, MemoryDocument, MemoryError, SearchResult};
use pgvector::Vector;
use sqlx::PgPool;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// PostgreSQL + pgvector backed long-term memory store.
///
/// Each document is embedded with a pluggable `Embedder` (e.g. OpenAI
/// text-embedding-3-small) and stored in a `pekko_vector_documents` table.
/// Semantic search uses cosine distance (`<=>` operator).
///
/// When no embedder is provided, falls back to PostgreSQL `ILIKE` full-text
/// search so the system works even without an OpenAI key.
pub struct PgVectorStore {
    pool:     Arc<PgPool>,
    embedder: Option<Arc<dyn Embedder>>,
}

impl PgVectorStore {
    pub fn new(pool: Arc<PgPool>, embedder: Option<Arc<dyn Embedder>>) -> Self {
        Self { pool, embedder }
    }

    /// Create the pgvector extension and the documents table.
    /// Call once at startup (idempotent).
    pub async fn migrate(pool: &PgPool) -> Result<(), sqlx::Error> {
        sqlx::query("CREATE EXTENSION IF NOT EXISTS vector")
            .execute(pool)
            .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS pekko_vector_documents (
                id          TEXT        PRIMARY KEY,
                content     TEXT        NOT NULL,
                source      TEXT        NOT NULL,
                agent_id    TEXT        NOT NULL,
                metadata    JSONB       NOT NULL DEFAULT '{}',
                embedding   vector(1536),
                created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )
            "#,
        )
        .execute(pool)
        .await?;

        // IVFFlat index for approximate nearest-neighbour search.
        // `lists` is set low (10) for small datasets; tune in production.
        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS pekko_vector_documents_embedding_idx
                ON pekko_vector_documents
                USING ivfflat (embedding vector_cosine_ops)
                WITH (lists = 10)
            "#,
        )
        .execute(pool)
        .await?;

        info!("PgVectorStore migration complete");
        Ok(())
    }
}

// ─── LongTermMemory impl ──────────────────────────────────────────────────────

#[async_trait]
impl LongTermMemory for PgVectorStore {
    async fn store(&self, doc: MemoryDocument) -> Result<String, MemoryError> {
        let metadata = serde_json::to_value(&doc.metadata)
            .map_err(|e| MemoryError::StorageError(e.to_string()))?;

        let embedding: Option<Vector> = match &self.embedder {
            Some(emb) => match emb.embed(&doc.content).await {
                Ok(v)  => Some(Vector::from(v)),
                Err(e) => {
                    warn!(doc_id = %doc.id, error = %e, "Embedding failed — storing without vector");
                    None
                }
            },
            None => None,
        };

        sqlx::query(
            r#"
            INSERT INTO pekko_vector_documents (id, content, source, agent_id, metadata, embedding)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (id) DO UPDATE SET
                content    = EXCLUDED.content,
                source     = EXCLUDED.source,
                agent_id   = EXCLUDED.agent_id,
                metadata   = EXCLUDED.metadata,
                embedding  = EXCLUDED.embedding,
                created_at = NOW()
            "#,
        )
        .bind(&doc.id)
        .bind(&doc.content)
        .bind(&doc.source)
        .bind(&doc.agent_id)
        .bind(&metadata)
        .bind(embedding)
        .execute(&*self.pool)
        .await
        .map_err(|e| MemoryError::StorageError(e.to_string()))?;

        info!(doc_id = %doc.id, source = %doc.source, "Document stored in PgVectorStore");
        Ok(doc.id)
    }

    async fn search(&self, query: &str, top_k: usize) -> Result<Vec<SearchResult>, MemoryError> {
        debug!(query = %query, top_k = top_k, "PgVectorStore search");

        // Vector search path
        if let Some(emb) = &self.embedder {
            match emb.embed(query).await {
                Ok(vec) => {
                    let q_vec = Vector::from(vec);
                    let rows = sqlx::query_as::<_, (String, String, String, f32)>(
                        r#"
                        SELECT id, content, source, (embedding <=> $1)::real AS distance
                        FROM pekko_vector_documents
                        WHERE embedding IS NOT NULL
                        ORDER BY distance
                        LIMIT $2
                        "#,
                    )
                    .bind(q_vec)
                    .bind(top_k as i64)
                    .fetch_all(&*self.pool)
                    .await
                    .map_err(|e| MemoryError::StorageError(e.to_string()))?;

                    return Ok(rows.into_iter().map(|(id, content, source, dist)| {
                        SearchResult {
                            id,
                            score: 1.0 - dist,   // cosine similarity = 1 - distance
                            content,
                            source,
                        }
                    }).collect());
                }
                Err(e) => {
                    warn!(error = %e, "Embedding failed during search — falling back to text search");
                }
            }
        }

        // Text-search fallback (no embedding or embedding failed)
        let like_pattern = format!("%{query}%");
        let rows = sqlx::query_as::<_, (String, String, String)>(
            r#"
            SELECT id, content, source
            FROM pekko_vector_documents
            WHERE content ILIKE $1
            ORDER BY created_at DESC
            LIMIT $2
            "#,
        )
        .bind(&like_pattern)
        .bind(top_k as i64)
        .fetch_all(&*self.pool)
        .await
        .map_err(|e| MemoryError::StorageError(e.to_string()))?;

        Ok(rows.into_iter().map(|(id, content, source)| SearchResult {
            id, score: 0.5, content, source,
        }).collect())
    }

    async fn delete(&self, doc_id: &str) -> Result<(), MemoryError> {
        let result = sqlx::query("DELETE FROM pekko_vector_documents WHERE id = $1")
            .bind(doc_id)
            .execute(&*self.pool)
            .await
            .map_err(|e| MemoryError::StorageError(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(MemoryError::NotFound(format!("Document {doc_id} not found")));
        }
        info!(doc_id = %doc_id, "Document deleted from PgVectorStore");
        Ok(())
    }
}
