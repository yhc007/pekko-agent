use async_trait::async_trait;
use crate::{AgentError, AgentState, ToolDefinition};
use crate::message::{UserQuery, AgentAction, AgentResponse, Observation};
use serde::{Deserialize, Serialize};

// ─── pekko-actor integration ────────────────────────────────────────────────
//
// Each AgentActor is also a first-class pekko_actor::Actor.
// Messages sent through the ActorSystem arrive as `AgentMessage` variants and
// are dispatched to the appropriate ReAct-loop method.
// ─────────────────────────────────────────────────────────────────────────────
pub use pekko_actor::{Actor, ActorContext, ActorRef, ActorSystem, Props};

/// Top-level message envelope accepted by every pekko_actor mailbox for agents.
#[derive(Debug)]
pub enum AgentMessage {
    /// A new user query to process (starts a full ReAct loop).
    Query(UserQuery),
    /// Internal: the reasoning step produced an action — execute it.
    Execute(AgentAction),
    /// Internal: observation(s) are ready — synthesise the final response.
    Respond(Vec<Observation>),
}

// ─── AgentActor trait ───────────────────────────────────────────────────────

/// Domain-level trait for AI agents (ReAct loop abstraction).
///
/// Each implementor also implements `pekko_actor::Actor<Message = AgentMessage>`
/// so it can be spawned inside an `ActorSystem` and receive messages through a
/// typed mailbox.
#[async_trait]
pub trait AgentActor: Actor<Message = AgentMessage> + Send + Sync {
    /// Unique agent identifier.
    fn agent_id(&self) -> &str;

    /// List of tools available to this agent.
    fn available_tools(&self) -> Vec<ToolDefinition>;

    /// System prompt that defines the agent's role / expertise.
    fn system_prompt(&self) -> String;

    /// Maximum reasoning iterations before giving up (guards infinite loops).
    fn max_iterations(&self) -> u32 {
        10
    }

    /// Reasoning step: UserQuery → LLM reasoning → AgentAction decision.
    async fn reason(&mut self, query: &UserQuery) -> Result<AgentAction, AgentError>;

    /// Action step: execute tool calls implied by the chosen AgentAction.
    async fn act(&mut self, action: &AgentAction) -> Result<Vec<Observation>, AgentError>;

    /// Response step: synthesise observations into a final AgentResponse.
    async fn respond(&mut self, observations: &[Observation]) -> Result<AgentResponse, AgentError>;

    /// Return current FSM state.
    fn current_state(&self) -> &AgentState;

    /// Perform a state transition.
    fn transition(&mut self, new_state: AgentState);
}

// ─── AgentInfo & AgentStatus ────────────────────────────────────────────────

/// Metadata exposed to the orchestrator / receptionist.
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
