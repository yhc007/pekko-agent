use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

#[cfg(feature = "postgres")]
use crate::pg_audit::PgAuditStore;

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
    #[cfg(feature = "postgres")]
    pg_store: Option<Arc<PgAuditStore>>,
}

impl AuditLogger {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: Arc::new(RwLock::new(VecDeque::with_capacity(max_entries))),
            max_entries,
            #[cfg(feature = "postgres")]
            pg_store: None,
        }
    }

    /// Attach a Postgres store for persistent dual-write.
    #[cfg(feature = "postgres")]
    pub fn with_pg(mut self, store: Arc<PgAuditStore>) -> Self {
        self.pg_store = Some(store);
        self
    }

    pub async fn log(&self, entry: AuditEntry) {
        info!(
            agent = %entry.agent_id,
            action = %entry.action,
            resource = %entry.resource,
            "Audit log"
        );

        // Postgres write (background task — never blocks the caller)
        #[cfg(feature = "postgres")]
        if let Some(store) = &self.pg_store {
            let store = store.clone();
            let entry_clone = entry.clone();
            tokio::spawn(async move {
                if let Err(e) = store.record(&entry_clone).await {
                    tracing::warn!(error = %e, "Audit log Postgres write failed");
                }
            });
        }

        // In-memory ring buffer
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

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(tenant: &str, agent: &str, action: &str) -> AuditEntry {
        AuditEntry {
            id:        Uuid::new_v4(),
            timestamp: Utc::now(),
            tenant_id: tenant.into(),
            agent_id:  agent.into(),
            action:    action.into(),
            resource:  "/api/test".into(),
            outcome:   AuditOutcome::Success,
            details:   serde_json::Value::Null,
        }
    }

    #[tokio::test]
    async fn log_and_query_all_entries() {
        let logger = AuditLogger::new(100);
        logger.log(entry("t1", "agent-a", "query")).await;
        logger.log(entry("t1", "agent-b", "workflow")).await;

        let all = logger.query(None, None, 10).await;
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn query_filter_by_tenant() {
        let logger = AuditLogger::new(100);
        logger.log(entry("t1", "a", "q1")).await;
        logger.log(entry("t2", "b", "q2")).await;
        logger.log(entry("t1", "c", "q3")).await;

        let t1 = logger.query(Some("t1"), None, 10).await;
        assert_eq!(t1.len(), 2);

        let t2 = logger.query(Some("t2"), None, 10).await;
        assert_eq!(t2.len(), 1);
    }

    #[tokio::test]
    async fn query_filter_by_agent() {
        let logger = AuditLogger::new(100);
        logger.log(entry("t1", "agent-a", "q")).await;
        logger.log(entry("t1", "agent-b", "q")).await;
        logger.log(entry("t1", "agent-a", "q2")).await;

        let by_agent = logger.query(None, Some("agent-a"), 10).await;
        assert_eq!(by_agent.len(), 2);
    }

    #[tokio::test]
    async fn ring_buffer_evicts_oldest_when_full() {
        let logger = AuditLogger::new(3);
        for i in 0..5u32 {
            logger.log(entry("t", "a", &format!("action-{i}"))).await;
        }
        let entries = logger.query(None, None, 10).await;
        assert_eq!(entries.len(), 3);
        // most-recent-first from query (reversed)
        assert!(entries[0].action.starts_with("action-4"));
    }

    #[tokio::test]
    async fn query_respects_limit() {
        let logger = AuditLogger::new(100);
        for i in 0..10u32 {
            logger.log(entry("t1", "a", &format!("a{i}"))).await;
        }
        let limited = logger.query(None, None, 3).await;
        assert_eq!(limited.len(), 3);
    }

    #[test]
    fn audit_outcome_serializes() {
        let outcomes = vec![
            AuditOutcome::Success,
            AuditOutcome::Failure("db error".into()),
            AuditOutcome::Denied("no permission".into()),
        ];
        for o in outcomes {
            let json = serde_json::to_string(&o).unwrap();
            let rt: AuditOutcome = serde_json::from_str(&json).unwrap();
            assert_eq!(format!("{o:?}"), format!("{rt:?}"));
        }
    }
}
