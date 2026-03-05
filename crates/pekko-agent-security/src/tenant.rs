use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TenantContext {
    pub tenant_id: String,
    pub tenant_name: String,
    pub isolation_level: IsolationLevel,
    pub resource_limits: ResourceLimits,
    pub metadata: HashMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum IsolationLevel {
    Shared,
    Dedicated,
    FullIsolation,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResourceLimits {
    pub max_agents: u32,
    pub max_tokens_per_day: u64,
    pub max_concurrent_requests: u32,
    pub max_storage_mb: u64,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_agents: 10,
            max_tokens_per_day: 100_000,
            max_concurrent_requests: 50,
            max_storage_mb: 1024,
        }
    }
}

pub struct TenantManager {
    tenants: HashMap<String, TenantContext>,
}

impl TenantManager {
    pub fn new() -> Self {
        Self {
            tenants: HashMap::new(),
        }
    }

    pub fn register_tenant(&mut self, ctx: TenantContext) {
        self.tenants.insert(ctx.tenant_id.clone(), ctx);
    }

    pub fn get_tenant(&self, tenant_id: &str) -> Option<&TenantContext> {
        self.tenants.get(tenant_id)
    }

    pub fn validate_request(&self, tenant_id: &str) -> Result<&TenantContext, String> {
        self.tenants.get(tenant_id)
            .ok_or_else(|| format!("Tenant {} not found", tenant_id))
    }
}
