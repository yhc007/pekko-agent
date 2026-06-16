//! Postgres-backed audit log store.
//!
//! Persists `AuditEntry` records to `pekko_audit_log` and supports
//! time-ranged, tenant-scoped queries for compliance reporting.
//!
//! Enable with `features = ["postgres"]` in the depending crate.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::audit::{AuditEntry, AuditOutcome};

// ── Row type returned by queries ──────────────────────────────────────────────

#[derive(sqlx::FromRow)]
struct PgAuditRow {
    id:        Uuid,
    timestamp: DateTime<Utc>,
    tenant_id: String,
    agent_id:  String,
    action:    String,
    resource:  String,
    outcome:   String,
    details:   serde_json::Value,
}

impl From<PgAuditRow> for AuditEntry {
    fn from(r: PgAuditRow) -> Self {
        AuditEntry {
            id:        r.id,
            timestamp: r.timestamp,
            tenant_id: r.tenant_id,
            agent_id:  r.agent_id,
            action:    r.action,
            resource:  r.resource,
            outcome:   parse_outcome(&r.outcome),
            details:   r.details,
        }
    }
}

fn outcome_to_str(o: &AuditOutcome) -> String {
    match o {
        AuditOutcome::Success        => "success".to_string(),
        AuditOutcome::Failure(msg)   => format!("failure:{msg}"),
        AuditOutcome::Denied(msg)    => format!("denied:{msg}"),
    }
}

fn parse_outcome(s: &str) -> AuditOutcome {
    if s == "success" {
        AuditOutcome::Success
    } else if let Some(msg) = s.strip_prefix("failure:") {
        AuditOutcome::Failure(msg.to_string())
    } else if let Some(msg) = s.strip_prefix("denied:") {
        AuditOutcome::Denied(msg.to_string())
    } else {
        AuditOutcome::Failure(s.to_string())
    }
}

// ── Query parameters ──────────────────────────────────────────────────────────

#[derive(Debug, Default, Clone)]
pub struct AuditQuery {
    pub tenant_id: Option<String>,
    pub agent_id:  Option<String>,
    /// Prefix filter: `"query"` matches `"query"`, `"query.stream"`, etc.
    pub action:    Option<String>,
    pub from:      Option<DateTime<Utc>>,
    pub to:        Option<DateTime<Utc>>,
    pub limit:     i64,
    pub offset:    i64,
}

impl AuditQuery {
    pub fn new() -> Self {
        Self { limit: 50, ..Default::default() }
    }
}

// ── PgAuditStore ──────────────────────────────────────────────────────────────

pub struct PgAuditStore {
    pool: Arc<PgPool>,
}

impl PgAuditStore {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }

    /// Create the audit log table and indexes if they do not yet exist.
    pub async fn migrate(pool: &PgPool) -> Result<(), sqlx::Error> {
        sqlx::query(r#"
            CREATE TABLE IF NOT EXISTS pekko_audit_log (
                id          UUID        PRIMARY KEY,
                timestamp   TIMESTAMPTZ NOT NULL,
                tenant_id   TEXT        NOT NULL,
                agent_id    TEXT        NOT NULL,
                action      TEXT        NOT NULL,
                resource    TEXT        NOT NULL,
                outcome     TEXT        NOT NULL,
                details     JSONB       NOT NULL DEFAULT '{}'
            )
        "#).execute(pool).await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS pekko_audit_tenant_ts_idx \
             ON pekko_audit_log (tenant_id, timestamp DESC)"
        ).execute(pool).await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS pekko_audit_agent_ts_idx \
             ON pekko_audit_log (agent_id, timestamp DESC)"
        ).execute(pool).await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS pekko_audit_action_idx \
             ON pekko_audit_log (action text_pattern_ops)"
        ).execute(pool).await?;

        tracing::info!("pekko_audit_log table ready");
        Ok(())
    }

    /// Persist a single audit entry (fire-and-forget safe via `tokio::spawn`).
    pub async fn record(&self, entry: &AuditEntry) -> Result<(), sqlx::Error> {
        sqlx::query(r#"
            INSERT INTO pekko_audit_log
                (id, timestamp, tenant_id, agent_id, action, resource, outcome, details)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (id) DO NOTHING
        "#)
        .bind(entry.id)
        .bind(entry.timestamp)
        .bind(&entry.tenant_id)
        .bind(&entry.agent_id)
        .bind(&entry.action)
        .bind(&entry.resource)
        .bind(outcome_to_str(&entry.outcome))
        .bind(&entry.details)
        .execute(&*self.pool)
        .await?;
        Ok(())
    }

    /// Query the audit log with optional filters.
    /// NULL-coalescing pattern: a None filter matches all rows.
    pub async fn query(&self, q: &AuditQuery) -> Result<Vec<AuditEntry>, sqlx::Error> {
        let action_prefix = q.action.as_deref().map(|a| format!("{a}%"));

        let rows = sqlx::query_as::<_, PgAuditRow>(r#"
            SELECT * FROM pekko_audit_log
            WHERE ($1::TEXT  IS NULL OR tenant_id = $1)
              AND ($2::TEXT  IS NULL OR agent_id  = $2)
              AND ($3::TEXT  IS NULL OR action LIKE $3)
              AND ($4::TIMESTAMPTZ IS NULL OR timestamp >= $4)
              AND ($5::TIMESTAMPTZ IS NULL OR timestamp <= $5)
            ORDER BY timestamp DESC
            LIMIT $6 OFFSET $7
        "#)
        .bind(&q.tenant_id)
        .bind(&q.agent_id)
        .bind(&action_prefix)
        .bind(q.from)
        .bind(q.to)
        .bind(q.limit.max(1).min(500))
        .bind(q.offset.max(0))
        .fetch_all(&*self.pool)
        .await?;

        Ok(rows.into_iter().map(AuditEntry::from).collect())
    }

    /// Count of entries matching the filters — useful for pagination.
    pub async fn count(&self, q: &AuditQuery) -> Result<i64, sqlx::Error> {
        let action_prefix = q.action.as_deref().map(|a| format!("{a}%"));

        let row: (i64,) = sqlx::query_as(r#"
            SELECT COUNT(*) FROM pekko_audit_log
            WHERE ($1::TEXT  IS NULL OR tenant_id = $1)
              AND ($2::TEXT  IS NULL OR agent_id  = $2)
              AND ($3::TEXT  IS NULL OR action LIKE $3)
              AND ($4::TIMESTAMPTZ IS NULL OR timestamp >= $4)
              AND ($5::TIMESTAMPTZ IS NULL OR timestamp <= $5)
        "#)
        .bind(&q.tenant_id)
        .bind(&q.agent_id)
        .bind(&action_prefix)
        .bind(q.from)
        .bind(q.to)
        .fetch_one(&*self.pool)
        .await?;

        Ok(row.0)
    }

    /// Summary: count per action, grouped — useful for a stats dashboard.
    pub async fn action_summary(
        &self,
        tenant_id: &str,
        limit: i64,
    ) -> Result<Vec<(String, i64)>, sqlx::Error> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            r#"
            SELECT action, COUNT(*) AS cnt
            FROM pekko_audit_log
            WHERE tenant_id = $1
            GROUP BY action
            ORDER BY cnt DESC
            LIMIT $2
            "#
        )
        .bind(tenant_id)
        .bind(limit)
        .fetch_all(&*self.pool)
        .await?;

        Ok(rows)
    }
}
