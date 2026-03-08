use crate::types::*;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::info;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum OpenAIError {
    #[error("API error: status={status}, body={body}")]
    ApiError { status: u16, body: String },
    #[error("Network error: {0}")]
    NetworkError(String),
    #[error("Parse error: {0}")]
    ParseError(String),
}

// ─── OpenAI API Types ───

#[derive(Serialize)]
struct OpenAIRequest {
    model: String,
    messages: Vec<OpenAIMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OpenAITool>>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Serialize, Deserialize, Clone)]
struct OpenAIMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OpenAIToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
struct OpenAIToolCall {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: OpenAIFunctionCall,
}

#[derive(Serialize, Deserialize, Clone)]
struct OpenAIFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Serialize)]
struct OpenAITool {
    #[serde(rename = "type")]
    tool_type: String,
    function: OpenAIFunction,
}

#[derive(Serialize)]
struct OpenAIFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Deserialize)]
struct OpenAIResponse {
    choices: Vec<OpenAIChoice>,
    usage: Option<OpenAIUsage>,
}

#[derive(Deserialize)]
struct OpenAIChoice {
    message: OpenAIMessage,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct OpenAIUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
}

// ─── Client ───

pub struct OpenAIClient {
    client: Client,
    api_key: String,
    model: String,
}

impl OpenAIClient {
    pub fn new(api_key: String, model: Option<String>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("Failed to build HTTP client");
        Self { 
            client, 
            api_key,
            model: model.unwrap_or_else(|| "gpt-4o".to_string()),
        }
    }

    pub async fn send_message(
        &self,
        request: &LlmRequest,
    ) -> Result<LlmResponse, OpenAIError> {
        // Build messages with system prompt
        let mut messages: Vec<OpenAIMessage> = vec![
            OpenAIMessage {
                role: "system".to_string(),
                content: Some(request.system_prompt.clone()),
                tool_calls: None,
                tool_call_id: None,
            }
        ];
        
        // Convert messages to OpenAI format
        for msg in &request.messages {
            let role = if msg.role == "user" { "user" } else { "assistant" };
            
            // Check if this is a tool result message
            let has_tool_results = msg.content.iter().any(|b| matches!(b, ContentBlock::ToolResult { .. }));
            
            if has_tool_results {
                // Tool results become separate messages
                for block in &msg.content {
                    if let ContentBlock::ToolResult { tool_use_id, content, .. } = block {
                        messages.push(OpenAIMessage {
                            role: "tool".to_string(),
                            content: Some(content.clone()),
                            tool_calls: None,
                            tool_call_id: Some(tool_use_id.clone()),
                        });
                    }
                }
            } else {
                // Regular message or assistant with tool calls
                let mut text_content = String::new();
                let mut tool_calls: Vec<OpenAIToolCall> = Vec::new();
                
                for block in &msg.content {
                    match block {
                        ContentBlock::Text { text } => {
                            text_content.push_str(text);
                        }
                        ContentBlock::ToolUse { id, name, input } => {
                            tool_calls.push(OpenAIToolCall {
                                id: id.clone(),
                                call_type: "function".to_string(),
                                function: OpenAIFunctionCall {
                                    name: name.clone(),
                                    arguments: serde_json::to_string(input).unwrap_or_default(),
                                },
                            });
                        }
                        _ => {}
                    }
                }
                
                messages.push(OpenAIMessage {
                    role: role.to_string(),
                    content: if text_content.is_empty() { None } else { Some(text_content) },
                    tool_calls: if tool_calls.is_empty() { None } else { Some(tool_calls) },
                    tool_call_id: None,
                });
            }
        }

        // Convert tools to OpenAI format
        let tools = if !request.tools.is_empty() {
            let openai_tools: Vec<OpenAITool> = request.tools
                .iter()
                .map(|t| {
                    let mut schema = t.input_schema.clone();
                    // Remove additionalProperties if present
                    if let Some(obj) = schema.as_object_mut() {
                        obj.remove("additionalProperties");
                    }
                    OpenAITool {
                        tool_type: "function".to_string(),
                        function: OpenAIFunction {
                            name: t.name.clone(),
                            description: t.description.clone(),
                            parameters: schema,
                        },
                    }
                })
                .collect();
            Some(openai_tools)
        } else {
            None
        };

        let openai_req = OpenAIRequest {
            model: self.model.clone(),
            messages,
            tools,
            max_tokens: request.max_tokens,
            temperature: request.temperature,
        };

        let resp = self.client
            .post("https://api.openai.com/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&openai_req)
            .send()
            .await
            .map_err(|e| OpenAIError::NetworkError(e.to_string()))?;

        let status = resp.status().as_u16();
        if status != 200 {
            let body = resp.text().await.unwrap_or_default();
            return Err(OpenAIError::ApiError { status, body });
        }

        let openai_resp: OpenAIResponse = resp
            .json()
            .await
            .map_err(|e| OpenAIError::ParseError(e.to_string()))?;

        // Convert to LlmResponse
        let choice = openai_resp.choices.first()
            .ok_or_else(|| OpenAIError::ParseError("No choices".to_string()))?;

        let mut content_blocks: Vec<ContentBlock> = Vec::new();

        // Add text content if present
        if let Some(ref text) = choice.message.content {
            if !text.is_empty() {
                content_blocks.push(ContentBlock::Text { text: text.clone() });
            }
        }

        // Add tool calls if present
        if let Some(ref tool_calls) = choice.message.tool_calls {
            for tc in tool_calls {
                let input: serde_json::Value = serde_json::from_str(&tc.function.arguments)
                    .unwrap_or(serde_json::json!({}));
                content_blocks.push(ContentBlock::ToolUse {
                    id: tc.id.clone(),
                    name: tc.function.name.clone(),
                    input,
                });
            }
        }

        let stop_reason = match choice.finish_reason.as_deref() {
            Some("stop") => "end_turn",
            Some("tool_calls") => "tool_use",
            Some("length") => "max_tokens",
            _ if choice.message.tool_calls.is_some() => "tool_use",
            _ => "end_turn",
        };

        let usage = openai_resp.usage.as_ref();
        
        info!(
            model = %self.model,
            input_tokens = usage.map(|u| u.prompt_tokens).unwrap_or(0),
            output_tokens = usage.map(|u| u.completion_tokens).unwrap_or(0),
            "OpenAI API call succeeded"
        );

        Ok(LlmResponse {
            content: content_blocks,
            stop_reason: stop_reason.to_string(),
            usage: ClaudeUsage {
                input_tokens: usage.map(|u| u.prompt_tokens).unwrap_or(0),
                output_tokens: usage.map(|u| u.completion_tokens).unwrap_or(0),
            },
        })
    }
}
