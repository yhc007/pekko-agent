//! Redis-backed response cache for the pekko-agent platform.
//!
//! Cache keys: `{prefix}:{agent_id}:{first16_of_sha256(tenant_id:query)}`
//!
//! Only stateless (session-less) queries are cached. TTL is configurable
//! globally or per-agent via `CacheConfig::agent_ttl_overrides`.

use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use redis::{aio::ConnectionManager, AsyncCommands};
use serde::Serialize;
use sha2::{Sha256, Digest};

// ── Configuration ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct CacheConfig {
    pub redis_url:            String,
    pub key_prefix:           String,
    pub default_ttl_seconds:  u64,
    /// Per-agent TTL overrides (seconds). Falls back to `default_ttl_seconds`.
    pub agent_ttl_overrides:  HashMap<String, u64>,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            redis_url:           "redis://127.0.0.1:6379".to_string(),
            key_prefix:          "pekko:response".to_string(),
            default_ttl_seconds: 300,
            agent_ttl_overrides: HashMap::new(),
        }
    }
}

// ── Statistics ────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct CacheStats {
    pub hits:    u64,
    pub misses:  u64,
    pub sets:    u64,
    pub deletes: u64,
    pub errors:  u64,
    pub hit_rate_pct: f64,
}

struct Counters {
    hits:    AtomicU64,
    misses:  AtomicU64,
    sets:    AtomicU64,
    deletes: AtomicU64,
    errors:  AtomicU64,
}

impl Counters {
    fn new() -> Self {
        Self {
            hits:    AtomicU64::new(0),
            misses:  AtomicU64::new(0),
            sets:    AtomicU64::new(0),
            deletes: AtomicU64::new(0),
            errors:  AtomicU64::new(0),
        }
    }

    fn snapshot(&self) -> CacheStats {
        let hits   = self.hits.load(Ordering::Relaxed);
        let misses = self.misses.load(Ordering::Relaxed);
        let total  = hits + misses;
        let hit_rate_pct = if total == 0 { 0.0 } else { hits as f64 / total as f64 * 100.0 };
        CacheStats {
            hits,
            misses,
            sets:    self.sets.load(Ordering::Relaxed),
            deletes: self.deletes.load(Ordering::Relaxed),
            errors:  self.errors.load(Ordering::Relaxed),
            hit_rate_pct,
        }
    }
}

// ── ResponseCache ─────────────────────────────────────────────────────────────

pub struct ResponseCache {
    manager: ConnectionManager,
    config:  CacheConfig,
    counters: Arc<Counters>,
}

impl ResponseCache {
    /// Connect to Redis and return a ready cache handle.
    pub async fn new(config: CacheConfig) -> anyhow::Result<Self> {
        let client  = redis::Client::open(config.redis_url.as_str())?;
        let manager = ConnectionManager::new(client).await?;
        Ok(Self { manager, config, counters: Arc::new(Counters::new()) })
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    fn cache_key(&self, agent_id: &str, tenant_id: &str, query: &str) -> String {
        let mut h = Sha256::new();
        h.update(tenant_id.as_bytes());
        h.update(b":");
        h.update(query.as_bytes());
        let digest = hex::encode(h.finalize());
        format!("{}:{}:{}", self.config.key_prefix, agent_id, &digest[..16])
    }

    fn ttl(&self, agent_id: &str) -> u64 {
        self.config.agent_ttl_overrides
            .get(agent_id)
            .copied()
            .unwrap_or(self.config.default_ttl_seconds)
    }

    // ── Public API ────────────────────────────────────────────────────────────

    /// Return cached JSON string if present, `None` on miss or Redis error.
    pub async fn get(&self, agent_id: &str, tenant_id: &str, query: &str) -> Option<String> {
        let key = self.cache_key(agent_id, tenant_id, query);
        let mut conn = self.manager.clone();
        match conn.get::<_, Option<String>>(key.as_str()).await {
            Ok(Some(v)) => {
                self.counters.hits.fetch_add(1, Ordering::Relaxed);
                tracing::debug!(key = %key, "Cache HIT");
                Some(v)
            }
            Ok(None) => {
                self.counters.misses.fetch_add(1, Ordering::Relaxed);
                None
            }
            Err(e) => {
                self.counters.errors.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(key = %key, error = %e, "Redis GET failed");
                None
            }
        }
    }

    /// Store a JSON string with TTL derived from config.
    pub async fn set(&self, agent_id: &str, tenant_id: &str, query: &str, value: &str) {
        let key = self.cache_key(agent_id, tenant_id, query);
        let ttl = self.ttl(agent_id);
        let mut conn = self.manager.clone();
        match conn.set_ex::<_, _, ()>(key.as_str(), value, ttl).await {
            Ok(()) => {
                self.counters.sets.fetch_add(1, Ordering::Relaxed);
                tracing::debug!(key = %key, ttl_secs = ttl, "Cache SET");
            }
            Err(e) => {
                self.counters.errors.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(key = %key, error = %e, "Redis SET failed");
            }
        }
    }

    /// Delete all cached entries for a specific agent.
    ///
    /// Uses `KEYS` (acceptable for development; replace with `SCAN` in very
    /// large deployments to avoid blocking Redis).
    pub async fn flush_agent(&self, agent_id: &str) -> u64 {
        let pattern = format!("{}:{}:*", self.config.key_prefix, agent_id);
        self.delete_by_pattern(&pattern).await
    }

    /// Delete all entries under this cache's key prefix.
    pub async fn flush_all(&self) -> u64 {
        let pattern = format!("{}:*", self.config.key_prefix);
        self.delete_by_pattern(&pattern).await
    }

    async fn delete_by_pattern(&self, pattern: &str) -> u64 {
        let mut conn = self.manager.clone();
        let keys: Vec<String> = match redis::cmd("KEYS")
            .arg(pattern)
            .query_async(&mut conn)
            .await
        {
            Ok(k) => k,
            Err(e) => {
                self.counters.errors.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(pattern = %pattern, error = %e, "Redis KEYS failed");
                return 0;
            }
        };

        if keys.is_empty() {
            return 0;
        }

        match conn.del::<_, u64>(keys.as_slice()).await {
            Ok(n) => {
                self.counters.deletes.fetch_add(n, Ordering::Relaxed);
                tracing::info!(pattern = %pattern, deleted = n, "Cache flushed");
                n
            }
            Err(e) => {
                self.counters.errors.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(pattern = %pattern, error = %e, "Redis DEL failed");
                0
            }
        }
    }

    /// Current hit/miss/error counters.
    pub fn stats(&self) -> CacheStats {
        self.counters.snapshot()
    }
}
