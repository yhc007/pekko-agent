use async_trait::async_trait;
use pekko_agent_core::{Embedder, MemoryError};
use serde::Deserialize;
use tracing::{debug, warn};

const DEFAULT_MODEL: &str = "text-embedding-3-small";
const DEFAULT_DIMS: usize = 1536;

pub struct EmbeddingClient {
    api_key: String,
    model:   String,
    client:  reqwest::Client,
}

impl EmbeddingClient {
    pub fn new(api_key: impl Into<String>, model: Option<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model:   model.unwrap_or_else(|| DEFAULT_MODEL.to_string()),
            client:  reqwest::Client::new(),
        }
    }
}

// ─── OpenAI embeddings API response ──────────────────────────────────────────

#[derive(Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[async_trait]
impl Embedder for EmbeddingClient {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, MemoryError> {
        debug!(model = %self.model, chars = text.len(), "Requesting embedding");

        let resp = self.client
            .post("https://api.openai.com/v1/embeddings")
            .bearer_auth(&self.api_key)
            .json(&serde_json::json!({
                "input": text,
                "model": self.model
            }))
            .send()
            .await
            .map_err(|e| MemoryError::StorageError(format!("Embedding HTTP error: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            warn!(status = %status, body = %body, "Embedding API error");
            return Err(MemoryError::StorageError(format!(
                "Embedding API {status}: {body}"
            )));
        }

        let body: EmbeddingResponse = resp
            .json()
            .await
            .map_err(|e| MemoryError::StorageError(format!("Embedding parse error: {e}")))?;

        body.data
            .into_iter()
            .next()
            .map(|d| d.embedding)
            .ok_or_else(|| MemoryError::StorageError("Empty embedding response".into()))
    }

    fn dims(&self) -> usize {
        DEFAULT_DIMS
    }
}
