use async_trait::async_trait;
use pekko_agent_core::{LongTermMemory, MemoryDocument, SearchResult, MemoryError};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, debug};

/// In-memory vector store for document storage and retrieval
/// 
/// Production deployments would use Qdrant, Pinecone, or similar vector databases
pub struct InMemoryVectorStore {
    documents: Arc<RwLock<HashMap<String, MemoryDocument>>>,
}

impl InMemoryVectorStore {
    /// Create a new in-memory vector store
    pub fn new() -> Self {
        Self {
            documents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get the number of stored documents
    pub async fn document_count(&self) -> usize {
        self.documents.read().await.len()
    }

    /// Check if a document exists
    pub async fn contains(&self, doc_id: &str) -> bool {
        self.documents.read().await.contains_key(doc_id)
    }

    /// Get a specific document by ID
    pub async fn get(&self, doc_id: &str) -> Result<MemoryDocument, MemoryError> {
        self.documents
            .read()
            .await
            .get(doc_id)
            .cloned()
            .ok_or_else(|| MemoryError::NotFound(format!("Document {} not found", doc_id)))
    }

    /// List all document IDs
    pub async fn list_documents(&self) -> Vec<String> {
        self.documents.read().await.keys().cloned().collect()
    }
}

#[async_trait]
impl LongTermMemory for InMemoryVectorStore {
    async fn store(&self, doc: MemoryDocument) -> Result<String, MemoryError> {
        let id = doc.id.clone();
        let mut store = self.documents.write().await;
        
        info!(
            doc_id = %id,
            source = %doc.source,
            "Storing document in vector store"
        );
        
        store.insert(id.clone(), doc);
        Ok(id)
    }

    async fn search(&self, query: &str, top_k: usize) -> Result<Vec<SearchResult>, MemoryError> {
        let store = self.documents.read().await;
        let query_lower = query.to_lowercase();

        debug!(query = %query, top_k = top_k, "Searching vector store");

        // Simple text-based search (production would use vector similarity)
        let mut results: Vec<SearchResult> = store
            .values()
            .filter_map(|doc| {
                let content_lower = doc.content.to_lowercase();
                if content_lower.contains(&query_lower) {
                    // Calculate a basic relevance score based on query position
                    let score = if let Some(pos) = content_lower.find(&query_lower) {
                        // Higher score if match is near the beginning
                        let position_factor = 1.0 - (pos as f64 / content_lower.len() as f64);
                        0.5 + (position_factor * 0.5)
                    } else {
                        0.5
                    };

                    Some(SearchResult {
                        id: doc.id.clone(),
                        score: score as f32,
                        content: doc.content.clone(),
                        source: doc.source.clone(),
                    })
                } else {
                    None
                }
            })
            .collect();

        // Sort by relevance score (highest first)
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Limit to top_k results
        results.truncate(top_k);

        debug!(
            query = %query,
            results = results.len(),
            "Search completed"
        );

        Ok(results)
    }

    async fn delete(&self, doc_id: &str) -> Result<(), MemoryError> {
        let mut store = self.documents.write().await;
        store.remove(doc_id).ok_or_else(|| {
            MemoryError::NotFound(format!("Document {} not found", doc_id))
        })?;
        info!(doc_id = %doc_id, "Deleted document from vector store");
        Ok(())
    }
}

impl Clone for InMemoryVectorStore {
    fn clone(&self) -> Self {
        Self {
            documents: Arc::clone(&self.documents),
        }
    }
}

impl Default for InMemoryVectorStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_vector_store_creation() {
        let store = InMemoryVectorStore::new();
        assert_eq!(store.document_count().await, 0);
    }

    fn test_doc(id: &str, content: &str) -> MemoryDocument {
        MemoryDocument {
            id: id.to_string(),
            content: content.to_string(),
            source: "test".to_string(),
            agent_id: "test-agent".to_string(),
            metadata: std::collections::HashMap::new(),
        }
    }

    #[tokio::test]
    async fn test_store_document() {
        let store = InMemoryVectorStore::new();
        let result = store.store(test_doc("doc-1", "This is a test document")).await;
        assert!(result.is_ok());
        assert_eq!(store.document_count().await, 1);
    }

    #[tokio::test]
    async fn test_search_documents() {
        let store = InMemoryVectorStore::new();
        let _ = store.store(test_doc("doc-1", "Environmental compliance report")).await;
        let _ = store.store(test_doc("doc-2", "Safety audit findings")).await;

        let results = store.search("compliance", 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "doc-1");
    }

    #[tokio::test]
    async fn test_search_multiple_results() {
        let store = InMemoryVectorStore::new();
        let _ = store.store(test_doc("doc-1", "Environmental safety compliance")).await;
        let _ = store.store(test_doc("doc-2", "Workplace safety procedures")).await;

        let results = store.search("safety", 10).await.unwrap();
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn test_delete_document() {
        let store = InMemoryVectorStore::new();
        let _ = store.store(test_doc("doc-1", "Test document")).await;
        assert_eq!(store.document_count().await, 1);

        let result = store.delete("doc-1").await;
        assert!(result.is_ok());
        assert_eq!(store.document_count().await, 0);
    }

    #[tokio::test]
    async fn test_delete_nonexistent() {
        let store = InMemoryVectorStore::new();
        let result = store.delete("nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_search_empty_store() {
        let store = InMemoryVectorStore::new();
        let results = store.search("anything", 10).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_contains() {
        let store = InMemoryVectorStore::new();
        let _ = store.store(test_doc("doc-1", "Test")).await;
        assert!(store.contains("doc-1").await);
        assert!(!store.contains("nonexistent").await);
    }

    #[tokio::test]
    async fn test_get_document() {
        let store = InMemoryVectorStore::new();
        let _ = store.store(test_doc("doc-1", "Test content")).await;
        let retrieved = store.get("doc-1").await.unwrap();
        assert_eq!(retrieved.content, "Test content");
    }

    #[tokio::test]
    async fn test_list_documents() {
        let store = InMemoryVectorStore::new();
        let _ = store.store(test_doc("doc-1", "Test 1")).await;
        let _ = store.store(test_doc("doc-2", "Test 2")).await;

        let docs = store.list_documents().await;
        assert_eq!(docs.len(), 2);
        assert!(docs.contains(&"doc-1".to_string()));
        assert!(docs.contains(&"doc-2".to_string()));
    }
}
