use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use uuid::Uuid;
use crate::error::ToolError;

/// MCP compatible tool definition
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub required_permissions: Vec<String>,
    pub timeout_ms: u64,
    pub idempotent: bool,
}

/// Tool execution interface
#[async_trait]
pub trait Tool: Send + Sync + 'static {
    fn definition(&self) -> ToolDefinition;

    async fn execute(
        &self,
        input: serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, ToolError>;

    fn validate_input(&self, input: &serde_json::Value) -> Result<(), ToolError> {
        let _ = input;
        Ok(())
    }
}

/// Tool execution context
#[derive(Clone, Debug)]
pub struct ToolContext {
    pub tenant_id: String,
    pub user_id: String,
    pub session_id: Uuid,
    pub credentials: HashMap<String, String>,
    pub timeout: Duration,
}

/// Tool execution output
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolOutput {
    pub content: serde_json::Value,
    pub metadata: HashMap<String, String>,
    pub is_error: bool,
}

impl ToolOutput {
    pub fn success(content: serde_json::Value) -> Self {
        Self { content, metadata: HashMap::new(), is_error: false }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            content: serde_json::json!({ "error": message.into() }),
            metadata: HashMap::new(),
            is_error: true,
        }
    }
}
