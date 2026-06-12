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

#[cfg(test)]
mod tests {
    use super::*;

    fn rl(default_rpm: u32) -> RateLimiter {
        RateLimiter::new(RateLimitConfig {
            admin_rpm:   1000,
            agent_rpm:   100,
            default_rpm,
        })
    }

    #[test]
    fn allows_requests_within_limit() {
        let limiter = rl(10);
        let roles   = vec!["agent".to_string()];
        for _ in 0..5 {
            assert!(limiter.check("tenant-a", &roles).is_ok());
        }
    }

    #[test]
    fn rejects_after_limit_exceeded() {
        let limiter = rl(3);
        let roles: Vec<String> = vec![];
        for _ in 0..3 {
            assert!(limiter.check("tenant-x", &roles).is_ok());
        }
        let err = limiter.check("tenant-x", &roles).unwrap_err();
        assert_eq!(err.limit, 3);
        assert!(err.retry_after_secs >= 1);
    }

    #[test]
    fn admin_gets_higher_limit_than_default() {
        let limiter = RateLimiter::new(RateLimitConfig {
            admin_rpm:   5,
            agent_rpm:   2,
            default_rpm: 1,
        });

        let admin_roles = vec!["admin".to_string()];
        for _ in 0..5 {
            assert!(limiter.check("admin-tenant", &admin_roles).is_ok());
        }
        assert!(limiter.check("admin-tenant", &admin_roles).is_err());
    }

    #[test]
    fn different_tenants_are_independent() {
        let limiter = rl(2);
        let roles: Vec<String> = vec![];

        limiter.check("tenant-a", &roles).unwrap();
        limiter.check("tenant-a", &roles).unwrap();
        assert!(limiter.check("tenant-a", &roles).is_err());

        // tenant-b is independent — should still be allowed
        assert!(limiter.check("tenant-b", &roles).is_ok());
    }

    #[test]
    fn current_count_tracks_requests() {
        let limiter = rl(100);
        assert_eq!(limiter.current_count("new-tenant"), 0);

        let roles = vec!["agent".to_string()];
        limiter.check("my-tenant", &roles).unwrap();
        limiter.check("my-tenant", &roles).unwrap();
        assert_eq!(limiter.current_count("my-tenant"), 2);
    }

    #[test]
    fn rate_limit_error_message_contains_tenant_and_limit() {
        let limiter = rl(1);
        let roles: Vec<String> = vec![];
        limiter.check("t1", &roles).unwrap();
        let err = limiter.check("t1", &roles).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("t1"));
        assert!(msg.contains('1'));
    }
}
