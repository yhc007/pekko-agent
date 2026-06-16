use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

pub struct ApiKeyStore {
    pool: Arc<PgPool>,
}

/// A stored API key (key_hash is never returned to callers).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct StoredApiKey {
    pub id:           Uuid,
    pub name:         String,
    pub user_id:      String,
    pub tenant_id:    String,
    pub roles:        Vec<String>,
    pub active:       bool,
    pub created_at:   DateTime<Utc>,
    pub expires_at:   Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
}

/// Returned exactly once when a key is created or rotated.
/// `raw_key` is the only time the plaintext key is exposed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyCreated {
    pub id:         Uuid,
    pub raw_key:    String,
    pub name:       String,
    pub user_id:    String,
    pub tenant_id:  String,
    pub roles:      Vec<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}

fn hash_key(raw: &str) -> String {
    let mut h = Sha256::new();
    h.update(raw.as_bytes());
    hex::encode(h.finalize())
}

fn generate_raw_key() -> String {
    // pak_ + UUID simple (32 hex chars) → 36 chars total
    format!("pak_{}", Uuid::new_v4().simple())
}

impl ApiKeyStore {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }

    pub async fn migrate(pool: &PgPool) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS pekko_api_keys (
                id           UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
                name         TEXT        NOT NULL,
                user_id      TEXT        NOT NULL,
                tenant_id    TEXT        NOT NULL,
                roles        TEXT[]      NOT NULL DEFAULT '{}',
                key_hash     TEXT        NOT NULL UNIQUE,
                active       BOOLEAN     NOT NULL DEFAULT TRUE,
                created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                expires_at   TIMESTAMPTZ,
                last_used_at TIMESTAMPTZ
            );
            CREATE INDEX IF NOT EXISTS pekko_api_keys_tenant_active
                ON pekko_api_keys (tenant_id, active);
            "#,
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    pub async fn create(
        &self,
        name:       &str,
        user_id:    &str,
        tenant_id:  &str,
        roles:      Vec<String>,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<ApiKeyCreated> {
        let raw      = generate_raw_key();
        let key_hash = hash_key(&raw);
        let id       = Uuid::new_v4();
        let now      = Utc::now();

        sqlx::query(
            r#"
            INSERT INTO pekko_api_keys
                (id, name, user_id, tenant_id, roles, key_hash, active, created_at, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6, TRUE, $7, $8)
            "#,
        )
        .bind(id)
        .bind(name)
        .bind(user_id)
        .bind(tenant_id)
        .bind(&roles)
        .bind(&key_hash)
        .bind(now)
        .bind(expires_at)
        .execute(&*self.pool)
        .await?;

        Ok(ApiKeyCreated {
            id,
            raw_key: raw,
            name:       name.to_string(),
            user_id:    user_id.to_string(),
            tenant_id:  tenant_id.to_string(),
            roles,
            created_at: now,
            expires_at,
        })
    }

    /// Verify a raw key.  Updates `last_used_at` on success.
    /// Returns `None` if the key is unknown, revoked, or expired.
    pub async fn verify(&self, raw_key: &str) -> Result<Option<StoredApiKey>> {
        let key_hash = hash_key(raw_key);

        let row: Option<StoredApiKey> = sqlx::query_as(
            r#"
            SELECT id, name, user_id, tenant_id, roles, active, created_at, expires_at, last_used_at
            FROM pekko_api_keys
            WHERE key_hash = $1
              AND active   = TRUE
              AND (expires_at IS NULL OR expires_at > NOW())
            "#,
        )
        .bind(&key_hash)
        .fetch_optional(&*self.pool)
        .await?;

        if let Some(ref key) = row {
            sqlx::query("UPDATE pekko_api_keys SET last_used_at = NOW() WHERE id = $1")
                .bind(key.id)
                .execute(&*self.pool)
                .await?;
        }

        Ok(row)
    }

    /// List all keys for a tenant (no key_hash exposed).
    pub async fn list(&self, tenant_id: Option<&str>) -> Result<Vec<StoredApiKey>> {
        let rows = sqlx::query_as::<_, StoredApiKey>(
            r#"
            SELECT id, name, user_id, tenant_id, roles, active, created_at, expires_at, last_used_at
            FROM pekko_api_keys
            WHERE ($1::TEXT IS NULL OR tenant_id = $1)
            ORDER BY created_at DESC
            "#,
        )
        .bind(tenant_id)
        .fetch_all(&*self.pool)
        .await?;
        Ok(rows)
    }

    /// Revoke a key by id.  Returns `true` if the key existed and was active.
    pub async fn revoke(&self, id: Uuid) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE pekko_api_keys SET active = FALSE WHERE id = $1 AND active = TRUE",
        )
        .bind(id)
        .execute(&*self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Atomically revoke `old_id` and issue a new key with the same metadata.
    /// Returns `None` if `old_id` is not found or already revoked.
    pub async fn rotate(&self, old_id: Uuid) -> Result<Option<ApiKeyCreated>> {
        let mut tx = self.pool.begin().await?;

        let old: Option<StoredApiKey> = sqlx::query_as(
            r#"
            SELECT id, name, user_id, tenant_id, roles, active, created_at, expires_at, last_used_at
            FROM pekko_api_keys
            WHERE id = $1 AND active = TRUE
            FOR UPDATE
            "#,
        )
        .bind(old_id)
        .fetch_optional(&mut *tx)
        .await?;

        let Some(old) = old else {
            return Ok(None);
        };

        // Revoke the old key
        sqlx::query("UPDATE pekko_api_keys SET active = FALSE WHERE id = $1")
            .bind(old_id)
            .execute(&mut *tx)
            .await?;

        // Create a new key with the same metadata
        let raw      = generate_raw_key();
        let key_hash = hash_key(&raw);
        let new_id   = Uuid::new_v4();
        let now      = Utc::now();

        sqlx::query(
            r#"
            INSERT INTO pekko_api_keys
                (id, name, user_id, tenant_id, roles, key_hash, active, created_at, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6, TRUE, $7, $8)
            "#,
        )
        .bind(new_id)
        .bind(&old.name)
        .bind(&old.user_id)
        .bind(&old.tenant_id)
        .bind(&old.roles)
        .bind(&key_hash)
        .bind(now)
        .bind(old.expires_at)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(Some(ApiKeyCreated {
            id:         new_id,
            raw_key:    raw,
            name:       old.name,
            user_id:    old.user_id,
            tenant_id:  old.tenant_id,
            roles:      old.roles,
            created_at: now,
            expires_at: old.expires_at,
        }))
    }
}
