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

/// Well-known event types
pub mod event_types {
    pub const TASK_ASSIGNED: &str = "task.assigned";
    pub const TASK_COMPLETED: &str = "task.completed";
    pub const TASK_FAILED: &str = "task.failed";
    pub const TOOL_EXECUTED: &str = "tool.executed";
    pub const LLM_CALLED: &str = "llm.called";
    pub const STATE_CHANGED: &str = "state.changed";
    pub const PERMIT_CREATED: &str = "ehs.permit.created";
    pub const INSPECTION_SCHEDULED: &str = "ehs.inspection.scheduled";
    pub const COMPLIANCE_CHECKED: &str = "ehs.compliance.checked";
}
