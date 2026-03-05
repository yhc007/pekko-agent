use async_trait::async_trait;
use crate::{AgentError, AgentState, ToolDefinition};
use crate::message::{UserQuery, AgentAction, AgentResponse, Observation};
use serde::{Deserialize, Serialize};

/// Base trait for all AI agents
/// Extends PersistentActor for event sourcing based state recovery
#[async_trait]
pub trait AgentActor: Send + Sync {
    /// Unique agent identifier
    fn agent_id(&self) -> &str;

    /// List of tools available to the agent
    fn available_tools(&self) -> Vec<ToolDefinition>;

    /// System prompt (defines agent role/expertise)
    fn system_prompt(&self) -> String;

    /// Maximum reasoning iterations (prevents infinite loops)
    fn max_iterations(&self) -> u32 {
        10
    }

    /// Reasoning step: user query → LLM reasoning → AgentAction decision
    async fn reason(&mut self, query: &UserQuery) -> Result<AgentAction, AgentError>;

    /// Action step: execute tool calls
    async fn act(&mut self, action: &AgentAction) -> Result<Vec<Observation>, AgentError>;

    /// Response step: synthesize observations → generate final response
    async fn respond(&mut self, observations: &[Observation]) -> Result<AgentResponse, AgentError>;

    /// Get current state
    fn current_state(&self) -> &AgentState;

    /// State transition
    fn transition(&mut self, new_state: AgentState);
}

/// Agent information (for orchestrator)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentInfo {
    pub agent_id: String,
    pub agent_type: String,
    pub description: String,
    pub capabilities: Vec<String>,
    pub status: AgentStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AgentStatus {
    Available,
    Busy,
    Offline,
    Error,
}
