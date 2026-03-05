use serde::{Deserialize, Serialize};
use crate::message::{ToolCall, Observation};

/// Agent state FSM
/// Idle → Reasoning → Acting → Observing → Responding → Idle
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AgentState {
    /// Idle state - ready for new queries
    Idle,
    /// Reasoning - LLM deciding next action
    Reasoning {
        query: String,
        iteration: u32,
        thought_chain: Vec<String>,
    },
    /// Acting - executing tool calls
    Acting {
        tool_calls: Vec<ToolCall>,
        pending: usize,
    },
    /// Observing - collecting & analyzing tool results
    Observing {
        observations: Vec<Observation>,
        needs_more: bool,
    },
    /// Generating response
    Responding {
        draft: String,
    },
    /// Error state
    Error {
        error: String,
        recoverable: bool,
    },
}

impl Default for AgentState {
    fn default() -> Self {
        Self::Idle
    }
}

impl AgentState {
    pub fn is_idle(&self) -> bool {
        matches!(self, Self::Idle)
    }

    pub fn is_busy(&self) -> bool {
        !matches!(self, Self::Idle | Self::Error { .. })
    }
}

/// Agent event (Event Sourcing)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AgentEvent {
    QueryReceived {
        session_id: uuid::Uuid,
        content: String,
        timestamp: chrono::DateTime<chrono::Utc>,
    },
    ReasoningStarted {
        iteration: u32,
        model: String,
    },
    ToolInvoked {
        call_id: String,
        tool_name: String,
        input: serde_json::Value,
    },
    ToolCompleted {
        call_id: String,
        output: serde_json::Value,
        duration_ms: u64,
    },
    ResponseGenerated {
        content: String,
        token_usage: crate::message::TokenUsage,
    },
    TaskDelegated {
        target_agent: String,
        task_id: uuid::Uuid,
    },
    ErrorOccurred {
        error: String,
        recoverable: bool,
    },
}
