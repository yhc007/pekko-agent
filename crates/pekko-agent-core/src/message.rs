use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};
use std::collections::HashMap;

/// User query
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserQuery {
    pub session_id: Uuid,
    pub content: String,
    pub context: ConversationContext,
    pub auth: AuthContext,
}

/// Conversation context
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConversationContext {
    pub messages: Vec<Message>,
    pub metadata: HashMap<String, String>,
}

/// Authentication context
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthContext {
    pub user_id: String,
    pub tenant_id: String,
    pub roles: Vec<String>,
}

/// Conversation message
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub role: MessageRole,
    pub content: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Tool,
}

impl std::fmt::Display for MessageRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MessageRole::User => write!(f, "user"),
            MessageRole::Assistant => write!(f, "assistant"),
            MessageRole::System => write!(f, "system"),
            MessageRole::Tool => write!(f, "tool"),
        }
    }
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: MessageRole::User, content: content.into(), timestamp: Utc::now() }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: MessageRole::Assistant, content: content.into(), timestamp: Utc::now() }
    }
}

/// Agent action decision
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AgentAction {
    /// Tool invocation
    UseTool(Vec<ToolCall>),
    /// Generate final response
    Respond(String),
    /// Delegate to another agent
    DelegateToAgent {
        target_agent: String,
        task: AgentTask,
    },
}

/// Tool call information
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

/// Agent task
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentTask {
    pub task_id: Uuid,
    pub description: String,
    pub input: serde_json::Value,
    pub priority: TaskPriority,
    pub timeout_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TaskPriority {
    Low,
    Normal,
    High,
    Critical,
}

/// Tool observation result
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Observation {
    pub tool_call_id: String,
    pub tool_name: String,
    pub result: serde_json::Value,
    pub is_error: bool,
    pub duration_ms: u64,
}

/// Agent final response
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentResponse {
    pub content: String,
    pub citations: Vec<Citation>,
    pub suggested_actions: Vec<String>,
    pub token_usage: TokenUsage,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Citation {
    pub source: String,
    pub text: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

impl TokenUsage {
    pub fn total(&self) -> u32 { self.input_tokens + self.output_tokens }
}

/// Inter-agent message
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AgentMessage {
    Query(UserQuery),
    ToolResult {
        call_id: String,
        tool_name: String,
        result: serde_json::Value,
        duration_ms: u64,
    },
    DelegatedTask {
        task_id: Uuid,
        from_agent: String,
        task: AgentTask,
    },
    OrchestrateCommand(OrchestrateCmd),
    SystemSignal(AgentSignal),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum OrchestrateCmd {
    AssignTask(AgentTask),
    CancelTask { task_id: Uuid },
    ReportProgress { task_id: Uuid },
    Shutdown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AgentSignal {
    HealthCheck,
    MemoryCompact,
    ConfigReload,
    GracefulShutdown,
}
