use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub tenant_id: String,
    pub agent_id: String,
    pub action: String,
    pub resource: String,
    pub outcome: AuditOutcome,
    pub details: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AuditOutcome {
    Success,
    Failure(String),
    Denied(String),
}

pub struct AuditLogger {
    entries: Arc<RwLock<VecDeque<AuditEntry>>>,
    max_entries: usize,
}

impl AuditLogger {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: Arc::new(RwLock::new(VecDeque::with_capacity(max_entries))),
            max_entries,
        }
    }

    pub async fn log(&self, entry: AuditEntry) {
        info!(
            agent = %entry.agent_id,
            action = %entry.action,
            resource = %entry.resource,
            "Audit log"
        );
        let mut entries = self.entries.write().await;
        if entries.len() >= self.max_entries {
            entries.pop_front();
        }
        entries.push_back(entry);
    }

    pub async fn query(
        &self,
        tenant_id: Option<&str>,
        agent_id: Option<&str>,
        limit: usize,
    ) -> Vec<AuditEntry> {
        let entries = self.entries.read().await;
        entries.iter()
            .rev()
            .filter(|e| tenant_id.map_or(true, |t| e.tenant_id == t))
            .filter(|e| agent_id.map_or(true, |a| e.agent_id == a))
            .take(limit)
            .cloned()
            .collect()
    }
}
