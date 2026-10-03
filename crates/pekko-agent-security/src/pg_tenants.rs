use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;

use crate::tenant::{IsolationLevel, ResourceLimits};

// ── Stored tenant (DB row) ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct StoredTenant {
    pub id:                      String,
    pub name:                    String,
    pub description:             Option<String>,
    pub owner_user_id:           String,
    pub isolation_level:         String,
    pub max_agents:              i32,
    pub max_tokens_per_day:      i64,
    pub max_concurrent_requests: i32,
    pub max_storage_mb:          i64,
    pub active:                  bool,
    pub created_at:              DateTime<Utc>,
    pub updated_at:              DateTime<Utc>,
}

impl StoredTenant {
    pub fn isolation_level(&self) -> IsolationLevel {
        match self.isolation_level.as_str() {
            "dedicated"      => IsolationLevel::Dedicated,
            "full_isolation" => IsolationLevel::FullIsolation,
            _                => IsolationLevel::Shared,
        }
    }

    pub fn resource_limits(&self) -> ResourceLimits {
        ResourceLimits {
            max_agents:               self.max_agents as u32,
            max_tokens_per_day:       self.max_tokens_per_day as u64,
            max_concurrent_requests:  self.max_concurrent_requests as u32,
            max_storage_mb:           self.max_storage_mb as u64,
        }
    }
}

fn isolation_to_str(level: &IsolationLevel) -> &'static str {
    match level {
        IsolationLevel::Shared        => "shared",
        IsolationLevel::Dedicated     => "dedicated",
        IsolationLevel::FullIsolation => "full_isolation",
    }
}

// ── Request DTOs ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateTenantRequest {
    pub id:              String,
    pub name:            String,
    pub description:     Option<String>,
    pub owner_user_id:   String,
    pub isolation_level: Option<IsolationLevel>,
    pub resource_limits: Option<ResourceLimits>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTenantRequest {
    pub name:            Option<String>,
    pub description:     Option<String>,
    pub isolation_level: Option<IsolationLevel>,
    pub resource_limits: Option<ResourceLimits>,
}

// ── Store ─────────────────────────────────────────────────────────────────────

pub struct PgTenantStore {
    pool: Arc<PgPool>,
}

impl PgTenantStore {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }

    pub async fn migrate(pool: &PgPool) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS pekko_tenants (
                id                      TEXT        PRIMARY KEY,
                name                    TEXT        NOT NULL,
                description             TEXT,
                owner_user_id           TEXT        NOT NULL,
                isolation_level         TEXT        NOT NULL DEFAULT 'shared',
                max_agents              INTEGER     NOT NULL DEFAULT 10,
                max_tokens_per_day      BIGINT      NOT NULL DEFAULT 100000,
                max_concurrent_requests INTEGER     NOT NULL DEFAULT 50,
                max_storage_mb          BIGINT      NOT NULL DEFAULT 1024,
                active                  BOOLEAN     NOT NULL DEFAULT TRUE,
                created_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at              TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )
            "#,
        )
        .execute(pool)
        .await?;

        // One statement per query(): sqlx uses the prepared-statement
        // protocol, which Postgres rejects for multi-command strings.
        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS pekko_tenants_active ON pekko_tenants (active)
            "#,
        )
        .execute(pool)
        .await?;

        Ok(())
    }

    pub async fn create(&self, req: CreateTenantRequest) -> Result<StoredTenant> {
        let limits    = req.resource_limits.unwrap_or_default();
        let isolation = req.isolation_level.unwrap_or(IsolationLevel::Shared);
        let now       = Utc::now();

        sqlx::query(
            r#"
            INSERT INTO pekko_tenants
                (id, name, description, owner_user_id, isolation_level,
                 max_agents, max_tokens_per_day, max_concurrent_requests,
                 max_storage_mb, active, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, TRUE, $10, $10)
            "#,
        )
        .bind(&req.id)
        .bind(&req.name)
        .bind(&req.description)
        .bind(&req.owner_user_id)
        .bind(isolation_to_str(&isolation))
        .bind(limits.max_agents              as i32)
        .bind(limits.max_tokens_per_day      as i64)
        .bind(limits.max_concurrent_requests as i32)
        .bind(limits.max_storage_mb          as i64)
        .bind(now)
        .execute(&*self.pool)
        .await?;

        self.get(&req.id).await?.ok_or_else(|| anyhow::anyhow!("Tenant not found after insert"))
    }

    pub async fn get(&self, id: &str) -> Result<Option<StoredTenant>> {
        let row = sqlx::query_as::<_, StoredTenant>(
            "SELECT * FROM pekko_tenants WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&*self.pool)
        .await?;
        Ok(row)
    }

    pub async fn get_active(&self, id: &str) -> Result<Option<StoredTenant>> {
        let row = sqlx::query_as::<_, StoredTenant>(
            "SELECT * FROM pekko_tenants WHERE id = $1 AND active = TRUE",
        )
        .bind(id)
        .fetch_optional(&*self.pool)
        .await?;
        Ok(row)
    }

    pub async fn list(&self, active_only: bool) -> Result<Vec<StoredTenant>> {
        let rows = sqlx::query_as::<_, StoredTenant>(
            r#"
            SELECT * FROM pekko_tenants
            WHERE ($1 = FALSE OR active = TRUE)
            ORDER BY created_at DESC
            "#,
        )
        .bind(active_only)
        .fetch_all(&*self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn update(&self, id: &str, req: UpdateTenantRequest) -> Result<Option<StoredTenant>> {
        let existing = match self.get(id).await? {
            Some(t) => t,
            None => return Ok(None),
        };

        // Destructure `existing` fully before using `req` to avoid partial-move issues
        let ex_limits = existing.resource_limits();
        let name        = req.name.unwrap_or(existing.name);
        let description = req.description.or(existing.description);
        let limits      = req.resource_limits.unwrap_or(ex_limits);
        let isolation   = req.isolation_level
            .map(|l| isolation_to_str(&l).to_string())
            .unwrap_or(existing.isolation_level);

        sqlx::query(
            r#"
            UPDATE pekko_tenants SET
                name                    = $2,
                description             = $3,
                isolation_level         = $4,
                max_agents              = $5,
                max_tokens_per_day      = $6,
                max_concurrent_requests = $7,
                max_storage_mb          = $8,
                updated_at              = NOW()
            WHERE id = $1
            "#,
        )
        .bind(id)
        .bind(&name)
        .bind(&description)
        .bind(&isolation)
        .bind(limits.max_agents              as i32)
        .bind(limits.max_tokens_per_day      as i64)
        .bind(limits.max_concurrent_requests as i32)
        .bind(limits.max_storage_mb          as i64)
        .execute(&*self.pool)
        .await?;

        self.get(id).await
    }

    /// Soft-delete: sets active = FALSE.  Returns true if the tenant existed.
    pub async fn deactivate(&self, id: &str) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE pekko_tenants SET active = FALSE, updated_at = NOW() WHERE id = $1 AND active = TRUE",
        )
        .bind(id)
        .execute(&*self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Re-enable a previously deactivated tenant.
    pub async fn activate(&self, id: &str) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE pekko_tenants SET active = TRUE, updated_at = NOW() WHERE id = $1",
        )
        .bind(id)
        .execute(&*self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }
}
