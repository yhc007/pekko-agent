use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::warn;

const WINDOW: Duration = Duration::from_secs(60);

/// Fixed-window rate limiter keyed by tenant_id.
///
/// Limits (requests per minute):
/// - Roles containing `"admin"` → `admin_rpm`
/// - Roles containing `"agent"` → `agent_rpm`
/// - Everything else            → `default_rpm`
#[derive(Clone)]
pub struct RateLimiter {
    windows: Arc<Mutex<HashMap<String, WindowState>>>,
    config:  RateLimitConfig,
}

#[derive(Clone, Copy, Debug)]
pub struct RateLimitConfig {
    pub admin_rpm:   u32,
    pub agent_rpm:   u32,
    pub default_rpm: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self { admin_rpm: 1000, agent_rpm: 300, default_rpm: 60 }
    }
}

#[derive(Debug)]
struct WindowState {
    count:        u32,
    window_start: Instant,
}

#[derive(Debug, thiserror::Error)]
#[error("Rate limit exceeded: {limit} req/min for tenant '{tenant_id}' — retry after {retry_after_secs}s")]
pub struct RateLimitError {
    pub tenant_id:        String,
    pub limit:            u32,
    pub retry_after_secs: u64,
}

impl RateLimiter {
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            windows: Arc::new(Mutex::new(HashMap::new())),
            config,
        }
    }

    /// Determine RPM limit for the given set of roles.
    fn limit_for(&self, roles: &[String]) -> u32 {
        if roles.iter().any(|r| r == "admin") {
            self.config.admin_rpm
        } else if roles.iter().any(|r| r == "agent") {
            self.config.agent_rpm
        } else {
            self.config.default_rpm
        }
    }

    /// Check whether `tenant_id` has capacity under the rate limit for `roles`.
    /// Increments the counter if allowed; returns `Err(RateLimitError)` if not.
    pub fn check(&self, tenant_id: &str, roles: &[String]) -> Result<(), RateLimitError> {
        let limit = self.limit_for(roles);
        let now   = Instant::now();

        let mut windows = self.windows.lock();
        let entry = windows.entry(tenant_id.to_string()).or_insert(WindowState {
            count:        0,
            window_start: now,
        });

        // Rotate window if expired
        if now.duration_since(entry.window_start) >= WINDOW {
            entry.count        = 0;
            entry.window_start = now;
        }

        if entry.count >= limit {
            let elapsed = now.duration_since(entry.window_start);
            let retry_after = WINDOW.saturating_sub(elapsed).as_secs().max(1);
            warn!(tenant_id, limit, count = entry.count, "Rate limit exceeded");
            return Err(RateLimitError {
                tenant_id:        tenant_id.to_string(),
                limit,
                retry_after_secs: retry_after,
            });
        }

        entry.count += 1;
        Ok(())
    }

    /// Current request count for a tenant within the active window (for metrics).
    pub fn current_count(&self, tenant_id: &str) -> u32 {
        let windows = self.windows.lock();
        match windows.get(tenant_id) {
            Some(w) if Instant::now().duration_since(w.window_start) < WINDOW => w.count,
            _ => 0,
        }
    }
}
