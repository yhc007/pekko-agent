use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentEventEnvelope {
    pub event_id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub source_service: String,
    pub event_type: String,
    pub tenant_id: String,
    pub correlation_id: Uuid,
    pub payload: serde_json::Value,
}

impl AgentEventEnvelope {
    pub fn new(
        source_service: impl Into<String>,
        event_type: impl Into<String>,
        tenant_id: impl Into<String>,
        correlation_id: Uuid,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            event_id: Uuid::new_v4(),
            timestamp: Utc::now(),
            source_service: source_service.into(),
            event_type: event_type.into(),
            tenant_id: tenant_id.into(),
            correlation_id,
            payload,
        }
    }

    pub fn topic_key(&self) -> String {
        format!("agent.{}", self.event_type)
    }
}

/// Well-known event types (dot-separated namespaces, prefix-matchable).
pub mod event_types {
    // ── Legacy ────────────────────────────────────────────────────────────────
    pub const TASK_ASSIGNED: &str = "task.assigned";
    pub const TASK_COMPLETED: &str = "task.completed";
    pub const TASK_FAILED: &str = "task.failed";
    pub const TOOL_EXECUTED: &str = "tool.executed";
    pub const LLM_CALLED: &str = "llm.called";
    pub const STATE_CHANGED: &str = "state.changed";

    // ── Agent queries ─────────────────────────────────────────────────────────
    pub const AGENT_QUERY_STARTED: &str   = "agent.query.started";
    pub const AGENT_QUERY_COMPLETED: &str = "agent.query.completed";
    pub const AGENT_QUERY_FAILED: &str    = "agent.query.failed";

    // ── Workflows ─────────────────────────────────────────────────────────────
    pub const WORKFLOW_STARTED: &str      = "workflow.started";
    pub const WORKFLOW_COMPLETED: &str    = "workflow.completed";
    pub const WORKFLOW_FAILED: &str       = "workflow.failed";
    pub const WORKFLOW_COMPENSATED: &str  = "workflow.compensated";

    // ── Multi-agent collaboration ─────────────────────────────────────────────
    pub const COLLABORATION_STARTED: &str   = "collaboration.started";
    pub const COLLABORATION_COMPLETED: &str = "collaboration.completed";

    // ── Saga ──────────────────────────────────────────────────────────────────
    pub const SAGA_STARTED: &str              = "saga.started";
    pub const SAGA_COMPLETED: &str            = "saga.completed";
    pub const SAGA_COMPENSATED: &str          = "saga.compensated";
    pub const SAGA_COMPENSATION_FAILED: &str  = "saga.compensation_failed";

    // ── EHS domain ───────────────────────────────────────────────────────────
    pub const PERMIT_CREATED: &str         = "ehs.permit.created";
    pub const INSPECTION_SCHEDULED: &str   = "ehs.inspection.scheduled";
    pub const COMPLIANCE_CHECKED: &str     = "ehs.compliance.checked";
}
