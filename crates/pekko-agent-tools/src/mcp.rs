use serde::{Deserialize, Serialize};
use pekko_agent_core::ToolDefinition;

/// MCP Tool Definition (Model Context Protocol)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// MCP Tool Call
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolCall {
    pub name: String,
    pub arguments: serde_json::Value,
}

/// MCP Tool Result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolResult {
    pub content: Vec<McpContent>,
    pub is_error: bool,
}

/// MCP Content block
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum McpContent {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { data: String, mime_type: String },
    #[serde(rename = "resource")]
    Resource { uri: String, text: String },
}

impl McpToolResult {
    /// Create a successful text result
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            content: vec![McpContent::Text { text: content.into() }],
            is_error: false,
        }
    }

    /// Create an error result
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            content: vec![McpContent::Text { text: message.into() }],
            is_error: true,
        }
    }

    /// Create a result with multiple content blocks
    pub fn with_content(content: Vec<McpContent>, is_error: bool) -> Self {
        Self { content, is_error }
    }
}

impl McpContent {
    /// Create a text content block
    pub fn text(text: impl Into<String>) -> Self {
        McpContent::Text { text: text.into() }
    }

    /// Create an image content block
    pub fn image(data: impl Into<String>, mime_type: impl Into<String>) -> Self {
        McpContent::Image {
            data: data.into(),
            mime_type: mime_type.into(),
        }
    }

    /// Create a resource content block
    pub fn resource(uri: impl Into<String>, text: impl Into<String>) -> Self {
        McpContent::Resource {
            uri: uri.into(),
            text: text.into(),
        }
    }
}

/// Convert from pekko ToolDefinition to MCP format
impl From<ToolDefinition> for McpToolDefinition {
    fn from(def: ToolDefinition) -> Self {
        Self {
            name: def.name,
            description: def.description,
            input_schema: def.input_schema,
        }
    }
}

/// Convert from MCP ToolDefinition to pekko format
impl From<McpToolDefinition> for ToolDefinition {
    fn from(mcp: McpToolDefinition) -> Self {
        Self {
            name: mcp.name,
            description: mcp.description,
            input_schema: mcp.input_schema,
            required_permissions: vec![],
            timeout_ms: 5000,
            idempotent: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_tool_result_text() {
        let result = McpToolResult::text("test");
        assert!(!result.is_error);
        assert_eq!(result.content.len(), 1);
    }

    #[test]
    fn test_mcp_tool_result_error() {
        let result = McpToolResult::error("error message");
        assert!(result.is_error);
        assert_eq!(result.content.len(), 1);
    }

    #[test]
    fn test_mcp_content_text() {
        let content = McpContent::text("hello");
        match content {
            McpContent::Text { text } => assert_eq!(text, "hello"),
            _ => panic!("Expected text content"),
        }
    }

    #[test]
    fn test_mcp_content_image() {
        let content = McpContent::image("data:image", "image/png");
        match content {
            McpContent::Image { data, mime_type } => {
                assert_eq!(data, "data:image");
                assert_eq!(mime_type, "image/png");
            }
            _ => panic!("Expected image content"),
        }
    }
}
