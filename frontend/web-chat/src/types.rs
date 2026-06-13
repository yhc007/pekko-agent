use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── Auth API ──

#[derive(Clone, Debug, Serialize)]
pub struct AuthRequest {
    pub api_key: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct AuthResponse {
    pub token: String,
    pub token_type: String,
    pub expires_in: u64,
    pub tenant_id: String,
    pub user_id: String,
    pub roles: Vec<String>,
}

// ── Collaborate API ──

#[derive(Clone, Debug, Serialize)]
pub struct CollaborateApiRequest {
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<Uuid>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct CollabAgentResponse {
    pub agent_id: String,
    pub response: String,
    pub tools_used: Vec<String>,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct CollaborationResult {
    pub session_id: Uuid,
    pub query: String,
    pub agent_responses: Vec<CollabAgentResponse>,
    pub synthesis: String,
    pub total_in_tokens: u32,
    pub total_out_tokens: u32,
}

// ── View Mode ──

#[derive(Clone, Debug, PartialEq)]
pub enum ViewMode {
    SingleAgent,
    Collaborate,
}

// ── Query API ──

#[derive(Clone, Debug, Serialize)]
pub struct QueryRequest {
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<Uuid>,
    pub tenant_id: String,
    pub user_id: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct QueryResponse {
    pub session_id: Uuid,
    pub agent_id: String,
    pub response: String,
    pub tools_used: Vec<String>,
    pub token_usage: TokenUsage,
}

#[derive(Clone, Debug, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

// ── Health API ──

#[derive(Clone, Debug, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub services: ServiceStatus,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ServiceStatus {
    pub orchestrator: String,
    pub tools_registered: usize,
    pub active_agents: usize,
}

// ── Agents API ──

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub status: String,
    pub tools: Vec<String>,
}

// ── Tools API ──

#[derive(Clone, Debug, Deserialize)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
}

// ── Session History API ──

#[derive(Clone, Debug, Deserialize)]
pub struct HistoryMessage {
    pub role: String,
    pub content: String,
    pub timestamp: String,
}

// ── Error ──

#[derive(Clone, Debug, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
    #[serde(default)]
    pub code: String,
}

// ── SSE Streaming events ──

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    Thinking { round: usize },
    ToolUse { tool: String, round: usize },
    ToolResult { tool: String, ok: bool },
    TextChunk { text: String },
    Done {
        session_id: Uuid,
        tools_used: Vec<String>,
        input_tokens: u32,
        output_tokens: u32,
    },
    Error { message: String },
}

// ── Internal Chat Message (for UI state) ──

#[derive(Clone, Debug, PartialEq)]
pub struct ChatMessage {
    pub id: String,
    pub role: MessageRole,
    pub content: String,
    pub tools_used: Vec<String>,
    pub token_usage: Option<TokenUsage>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub collaboration_result: Option<CollaborationResult>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

impl PartialEq for TokenUsage {
    fn eq(&self, other: &Self) -> bool {
        self.input_tokens == other.input_tokens && self.output_tokens == other.output_tokens
    }
}

// ── Agent metadata for sidebar ──

#[derive(Clone, Debug, PartialEq)]
pub struct AgentMeta {
    pub id: String,
    pub name: String,
    pub description: String,
    pub icon: &'static str,
    pub css_class: &'static str,
}

impl AgentMeta {
    pub fn defaults() -> Vec<Self> {
        vec![
            AgentMeta {
                id: "ehs-permit-agent".into(),
                name: "허가 관리".into(),
                description: "위험작업 허가/승인 관리".into(),
                icon: "",
                css_class: "permit",
            },
            AgentMeta {
                id: "ehs-inspection-agent".into(),
                name: "점검 관리".into(),
                description: "안전점검/시설점검 관리".into(),
                icon: "",
                css_class: "inspection",
            },
            AgentMeta {
                id: "ehs-compliance-agent".into(),
                name: "규정 준수".into(),
                description: "법규/규정 준수 확인".into(),
                icon: "",
                css_class: "compliance",
            },
        ]
    }
}
