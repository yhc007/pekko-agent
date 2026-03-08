use crate::types::*;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{info, warn};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum GeminiError {
    #[error("API error: status={status}, body={body}")]
    ApiError { status: u16, body: String },
    #[error("Network error: {0}")]
    NetworkError(String),
    #[error("Parse error: {0}")]
    ParseError(String),
}

// ─── Gemini API Types ───

#[derive(Serialize)]
struct GeminiRequest {
    contents: Vec<GeminiContent>,
    tools: Option<Vec<GeminiTool>>,
    #[serde(rename = "systemInstruction")]
    system_instruction: Option<GeminiContent>,
    #[serde(rename = "generationConfig")]
    generation_config: Option<GeminiGenerationConfig>,
}

#[derive(Serialize, Deserialize, Clone)]
struct GeminiContent {
    role: String,
    parts: Vec<GeminiPart>,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(untagged)]
enum GeminiPart {
    Text { text: String },
    FunctionCall { 
        #[serde(rename = "functionCall")]
        function_call: GeminiFunctionCall 
    },
    FunctionResponse { 
        #[serde(rename = "functionResponse")]
        function_response: GeminiFunctionResponse 
    },
}

#[derive(Serialize, Deserialize, Clone)]
struct GeminiFunctionCall {
    name: String,
    args: serde_json::Value,
}

#[derive(Serialize, Deserialize, Clone)]
struct GeminiFunctionResponse {
    name: String,
    response: serde_json::Value,
}

#[derive(Serialize)]
struct GeminiTool {
    #[serde(rename = "functionDeclarations")]
    function_declarations: Vec<GeminiFunctionDeclaration>,
}

#[derive(Serialize)]
struct GeminiFunctionDeclaration {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Serialize)]
struct GeminiGenerationConfig {
    temperature: Option<f32>,
    #[serde(rename = "maxOutputTokens")]
    max_output_tokens: u32,
}

#[derive(Deserialize)]
struct GeminiResponse {
    candidates: Vec<GeminiCandidate>,
    #[serde(rename = "usageMetadata")]
    usage_metadata: Option<GeminiUsageMetadata>,
}

#[derive(Deserialize)]
struct GeminiCandidate {
    content: GeminiContent,
    #[serde(rename = "finishReason")]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct GeminiUsageMetadata {
    #[serde(rename = "promptTokenCount")]
    prompt_token_count: Option<u32>,
    #[serde(rename = "candidatesTokenCount")]
    candidates_token_count: Option<u32>,
}

// ─── Client ───

pub struct GeminiClient {
    client: Client,
    api_key: String,
    model: String,
}

impl GeminiClient {
    pub fn new(api_key: String, model: Option<String>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("Failed to build HTTP client");
        Self { 
            client, 
            api_key,
            model: model.unwrap_or_else(|| "gemini-2.0-flash".to_string()),
        }
    }

    pub async fn send_message(
        &self,
        request: &LlmRequest,
    ) -> Result<LlmResponse, GeminiError> {
        // Convert messages to Gemini format
        let mut contents: Vec<GeminiContent> = Vec::new();
        
        for msg in &request.messages {
            let role = if msg.role == "user" { "user" } else { "model" };
            let mut parts: Vec<GeminiPart> = Vec::new();
            
            for block in &msg.content {
                match block {
                    ContentBlock::Text { text } => {
                        parts.push(GeminiPart::Text { text: text.clone() });
                    }
                    ContentBlock::ToolUse { id: _, name, input } => {
                        parts.push(GeminiPart::FunctionCall {
                            function_call: GeminiFunctionCall {
                                name: name.clone(),
                                args: input.clone(),
                            }
                        });
                    }
                    ContentBlock::ToolResult { tool_use_id: _, content, is_error: _ } => {
                        // Tool results go as function responses
                        parts.push(GeminiPart::FunctionResponse {
                            function_response: GeminiFunctionResponse {
                                name: "tool_result".to_string(),
                                response: serde_json::json!({ "result": content }),
                            }
                        });
                    }
                }
            }
            
            if !parts.is_empty() {
                contents.push(GeminiContent { role: role.to_string(), parts });
            }
        }

        // Convert tools to Gemini format (remove unsupported fields)
        let tools = if !request.tools.is_empty() {
            let declarations: Vec<GeminiFunctionDeclaration> = request.tools
                .iter()
                .map(|t| {
                    // Remove additionalProperties from schema (Gemini doesn't support it)
                    let mut schema = t.input_schema.clone();
                    if let Some(obj) = schema.as_object_mut() {
                        obj.remove("additionalProperties");
                        // Also remove from nested properties
                        if let Some(props) = obj.get_mut("properties") {
                            if let Some(props_obj) = props.as_object_mut() {
                                for (_, v) in props_obj.iter_mut() {
                                    if let Some(v_obj) = v.as_object_mut() {
                                        v_obj.remove("additionalProperties");
                                    }
                                }
                            }
                        }
                    }
                    GeminiFunctionDeclaration {
                        name: t.name.clone(),
                        description: t.description.clone(),
                        parameters: schema,
                    }
                })
                .collect();
            Some(vec![GeminiTool { function_declarations: declarations }])
        } else {
            None
        };

        let gemini_req = GeminiRequest {
            contents,
            tools,
            system_instruction: Some(GeminiContent {
                role: "user".to_string(),
                parts: vec![GeminiPart::Text { text: request.system_prompt.clone() }],
            }),
            generation_config: Some(GeminiGenerationConfig {
                temperature: request.temperature,
                max_output_tokens: request.max_tokens,
            }),
        };

        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            self.model, self.api_key
        );

        let resp = self.client
            .post(&url)
            .json(&gemini_req)
            .send()
            .await
            .map_err(|e| GeminiError::NetworkError(e.to_string()))?;

        let status = resp.status().as_u16();
        if status != 200 {
            let body = resp.text().await.unwrap_or_default();
            return Err(GeminiError::ApiError { status, body });
        }

        let gemini_resp: GeminiResponse = resp
            .json()
            .await
            .map_err(|e| GeminiError::ParseError(e.to_string()))?;

        // Convert to LlmResponse
        let candidate = gemini_resp.candidates.first()
            .ok_or_else(|| GeminiError::ParseError("No candidates".to_string()))?;

        let mut content_blocks: Vec<ContentBlock> = Vec::new();
        let mut tool_use_id = 0;

        for part in &candidate.content.parts {
            match part {
                GeminiPart::Text { text } => {
                    content_blocks.push(ContentBlock::Text { text: text.clone() });
                }
                GeminiPart::FunctionCall { function_call } => {
                    tool_use_id += 1;
                    content_blocks.push(ContentBlock::ToolUse {
                        id: format!("gemini_tool_{}", tool_use_id),
                        name: function_call.name.clone(),
                        input: function_call.args.clone(),
                    });
                }
                _ => {}
            }
        }

        let stop_reason = match candidate.finish_reason.as_deref() {
            Some("STOP") => "end_turn",
            Some("MAX_TOKENS") => "max_tokens",
            _ if content_blocks.iter().any(|b| matches!(b, ContentBlock::ToolUse { .. })) => "tool_use",
            _ => "end_turn",
        };

        let usage = gemini_resp.usage_metadata.as_ref();
        
        info!(
            model = %self.model,
            input_tokens = usage.and_then(|u| u.prompt_token_count).unwrap_or(0),
            output_tokens = usage.and_then(|u| u.candidates_token_count).unwrap_or(0),
            "Gemini API call succeeded"
        );

        Ok(LlmResponse {
            content: content_blocks,
            stop_reason: stop_reason.to_string(),
            usage: ClaudeUsage {
                input_tokens: usage.and_then(|u| u.prompt_token_count).unwrap_or(0),
                output_tokens: usage.and_then(|u| u.candidates_token_count).unwrap_or(0),
            },
        })
    }
}
