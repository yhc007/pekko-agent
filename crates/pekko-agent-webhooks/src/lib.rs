//! Webhook notification system for the pekko-agent platform.
//!
//! ## Architecture
//! - `WebhookRegistry` — Postgres-backed CRUD for subscriptions + delivery log
//! - `WebhookDeliverer` — async HTTP delivery with HMAC-SHA256 signing + retry
//! - `WebhookBridge`   — subscribes to EventPublisher and dispatches matching webhooks
//!
//! ## Matching
//! Subscription `event_types` are prefix-matched: a pattern `"agent.query"` fires on
//! `"agent.query.started"`, `"agent.query.completed"`, etc.
//!
//! ## Security
//! When a subscription has a `secret`, every delivery includes
//! `X-Pekko-Signature: sha256=<hmac_hex>` computed over the raw JSON body.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use sqlx::PgPool;
use uuid::Uuid;
use serde::{Deserialize, Serialize};

use pekko_agent_events::{AgentEventEnvelope, EventReceiver};

type HmacSha256 = Hmac<Sha256>;

// ── Domain types ──────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct WebhookSubscription {
    pub id:          Uuid,
    pub tenant_id:   String,
    pub url:         String,
    pub event_types: Vec<String>,
    pub secret:      Option<String>,
    pub active:      bool,
    pub created_at:  DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct WebhookDeliveryRecord {
    pub id:              Uuid,
    pub subscription_id: Uuid,
    pub event_id:        Uuid,
    pub event_type:      String,
    pub status:          String,
    pub response_code:   Option<i32>,
    pub attempt_count:   i32,
    pub delivered_at:    DateTime<Utc>,
}

// ── WebhookRegistry ───────────────────────────────────────────────────────────

pub struct WebhookRegistry {
    pool: Arc<PgPool>,
}

impl WebhookRegistry {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }

    /// Create the two webhook tables if they do not yet exist.
    pub async fn migrate(pool: &PgPool) -> Result<(), sqlx::Error> {
        sqlx::query(r#"
            CREATE TABLE IF NOT EXISTS pekko_webhook_subscriptions (
                id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
                tenant_id   TEXT        NOT NULL,
                url         TEXT        NOT NULL,
                event_types TEXT[]      NOT NULL,
                secret      TEXT,
                active      BOOLEAN     NOT NULL DEFAULT true,
                created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )
        "#).execute(pool).await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS pekko_wh_subs_tenant_idx \
             ON pekko_webhook_subscriptions (tenant_id, active)"
        ).execute(pool).await?;

        sqlx::query(r#"
            CREATE TABLE IF NOT EXISTS pekko_webhook_deliveries (
                id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
                subscription_id UUID        NOT NULL,
                event_id        UUID        NOT NULL,
                event_type      TEXT        NOT NULL,
                status          TEXT        NOT NULL,
                response_code   INT,
                attempt_count   INT         NOT NULL DEFAULT 1,
                delivered_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )
        "#).execute(pool).await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS pekko_wh_deliveries_sub_idx \
             ON pekko_webhook_deliveries (subscription_id, delivered_at DESC)"
        ).execute(pool).await?;

        tracing::info!("Webhook tables ready");
        Ok(())
    }

    /// Register a new webhook subscription. Returns the created record.
    pub async fn create(
        &self,
        tenant_id: &str,
        url: &str,
        event_types: Vec<String>,
        secret: Option<String>,
    ) -> Result<WebhookSubscription, sqlx::Error> {
        sqlx::query_as::<_, WebhookSubscription>(
            r#"
            INSERT INTO pekko_webhook_subscriptions (tenant_id, url, event_types, secret)
            VALUES ($1, $2, $3, $4)
            RETURNING *
            "#,
        )
        .bind(tenant_id)
        .bind(url)
        .bind(event_types)
        .bind(secret)
        .fetch_one(&*self.pool)
        .await
    }

    /// List all subscriptions for a tenant (active and inactive), newest first.
    pub async fn list(&self, tenant_id: &str) -> Result<Vec<WebhookSubscription>, sqlx::Error> {
        sqlx::query_as::<_, WebhookSubscription>(
            "SELECT * FROM pekko_webhook_subscriptions \
             WHERE tenant_id = $1 ORDER BY created_at DESC",
        )
        .bind(tenant_id)
        .fetch_all(&*self.pool)
        .await
    }

    /// Delete a subscription. Returns `true` if a row was deleted.
    pub async fn delete(&self, id: Uuid, tenant_id: &str) -> Result<bool, sqlx::Error> {
        let res = sqlx::query(
            "DELETE FROM pekko_webhook_subscriptions WHERE id = $1 AND tenant_id = $2",
        )
        .bind(id)
        .bind(tenant_id)
        .execute(&*self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Return active subscriptions whose `event_types` patterns prefix-match `event_type`.
    pub async fn find_matching(
        &self,
        event_type: &str,
        tenant_id: &str,
    ) -> Result<Vec<WebhookSubscription>, sqlx::Error> {
        let subs = sqlx::query_as::<_, WebhookSubscription>(
            "SELECT * FROM pekko_webhook_subscriptions \
             WHERE tenant_id = $1 AND active = true",
        )
        .bind(tenant_id)
        .fetch_all(&*self.pool)
        .await?;

        Ok(subs
            .into_iter()
            .filter(|s| {
                s.event_types.iter().any(|pattern| {
                    event_type == pattern
                        || event_type.starts_with(&format!("{pattern}."))
                })
            })
            .collect())
    }

    /// Return the last `limit` delivery records for a subscription.
    pub async fn list_deliveries(
        &self,
        subscription_id: Uuid,
        limit: i64,
    ) -> Result<Vec<WebhookDeliveryRecord>, sqlx::Error> {
        sqlx::query_as::<_, WebhookDeliveryRecord>(
            "SELECT * FROM pekko_webhook_deliveries \
             WHERE subscription_id = $1 \
             ORDER BY delivered_at DESC LIMIT $2",
        )
        .bind(subscription_id)
        .bind(limit)
        .fetch_all(&*self.pool)
        .await
    }

    pub(crate) async fn record_delivery(
        &self,
        subscription_id: Uuid,
        event: &AgentEventEnvelope,
        status: &str,
        response_code: Option<i32>,
        attempt_count: i32,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO pekko_webhook_deliveries
                (subscription_id, event_id, event_type, status, response_code, attempt_count)
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind(subscription_id)
        .bind(event.event_id)
        .bind(&event.event_type)
        .bind(status)
        .bind(response_code)
        .bind(attempt_count)
        .execute(&*self.pool)
        .await?;
        Ok(())
    }
}

// ── WebhookDeliverer ──────────────────────────────────────────────────────────

/// Delivers a single event to a webhook URL with up to 3 attempts and
/// exponential backoff (1 s → 2 s → 4 s).
pub struct WebhookDeliverer {
    client:   reqwest::Client,
    registry: Arc<WebhookRegistry>,
}

impl WebhookDeliverer {
    pub fn new(registry: Arc<WebhookRegistry>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(concat!(
                "pekko-agent-webhook/",
                env!("CARGO_PKG_VERSION"),
            ))
            .build()
            .expect("Failed to build webhook HTTP client");
        Self { client, registry }
    }

    pub async fn deliver(&self, sub: &WebhookSubscription, event: &AgentEventEnvelope) {
        let body = match serde_json::to_vec(event) {
            Ok(b) => b,
            Err(e) => {
                tracing::error!(error = %e, "Failed to serialise event for webhook");
                return;
            }
        };

        let signature = sub.secret.as_deref().map(|s| {
            format!("sha256={}", hmac_sha256_hex(s.as_bytes(), &body))
        });

        let retry_delays: [u64; 3] = [1, 2, 4];
        let mut attempt      = 0i32;
        let mut final_status = "failed";
        let mut final_code: Option<i32> = None;

        'retry: for (idx, delay_secs) in retry_delays.iter().enumerate() {
            attempt = (idx + 1) as i32;

            let mut builder = self.client
                .post(&sub.url)
                .header("content-type", "application/json")
                .header("x-pekko-event-type", &event.event_type)
                .header("x-pekko-event-id",   event.event_id.to_string());

            if let Some(sig) = &signature {
                builder = builder.header("x-pekko-signature", sig);
            }

            match builder.body(body.clone()).send().await {
                Ok(resp) => {
                    let code = resp.status().as_u16() as i32;
                    final_code = Some(code);
                    if resp.status().is_success() {
                        tracing::info!(
                            url = %sub.url, event_type = %event.event_type,
                            attempt, "Webhook delivered"
                        );
                        final_status = "delivered";
                        break 'retry;
                    }
                    tracing::warn!(
                        url = %sub.url, http_status = code, attempt,
                        "Webhook non-2xx — will retry"
                    );
                }
                Err(e) => {
                    tracing::warn!(url = %sub.url, error = %e, attempt, "Webhook request failed");
                }
            }

            if idx < retry_delays.len() - 1 {
                tokio::time::sleep(Duration::from_secs(*delay_secs)).await;
            }
        }

        if let Err(e) = self
            .registry
            .record_delivery(sub.id, event, final_status, final_code, attempt)
            .await
        {
            tracing::warn!(error = %e, "Failed to persist webhook delivery record");
        }
    }
}

fn hmac_sha256_hex(key: &[u8], data: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key size");
    mac.update(data);
    hex::encode(mac.finalize().into_bytes())
}

// ── WebhookBridge ─────────────────────────────────────────────────────────────

/// Bridges the EventPublisher broadcast channel to the webhook delivery system.
///
/// Call `WebhookBridge::start(...)` once during server startup; it spawns a
/// background task that runs for the lifetime of the process.
pub struct WebhookBridge;

impl WebhookBridge {
    /// Start the bridge as a detached Tokio task.
    pub fn start(
        registry:  Arc<WebhookRegistry>,
        deliverer: Arc<WebhookDeliverer>,
        mut receiver: EventReceiver<AgentEventEnvelope>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                match receiver.recv().await {
                    Ok(event) => {
                        let reg = registry.clone();
                        let del = deliverer.clone();
                        tokio::spawn(async move {
                            match reg
                                .find_matching(&event.event_type, &event.tenant_id)
                                .await
                            {
                                Ok(subs) if !subs.is_empty() => {
                                    tracing::debug!(
                                        event_type = %event.event_type,
                                        subscriptions = subs.len(),
                                        "Dispatching webhooks"
                                    );
                                    for sub in subs {
                                        del.deliver(&sub, &event).await;
                                    }
                                }
                                Ok(_) => {}
                                Err(e) => {
                                    tracing::warn!(
                                        error = %e,
                                        "Webhook subscription lookup failed"
                                    );
                                }
                            }
                        });
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(skipped = n, "Webhook bridge lagged — events dropped");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        tracing::info!("Webhook bridge: channel closed, shutting down");
                        break;
                    }
                }
            }
        })
    }
}
