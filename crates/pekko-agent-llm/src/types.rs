use serde::{Deserialize, Serialize};

/// Claude API request
#[derive(Debug, Serialize)]
pub struct ClaudeRequest {
    pub model: String,
    pub max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub messages: Vec<ClaudeMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ClaudeTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ClaudeMessage {
    pub role: String,
    pub content: Vec<ContentBlock>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
    /// 확장 사고 블록. 최신 모델(adaptive thinking 기본)은 표시 생략 시 빈 `thinking`과
    /// `signature`만 보냄. Tool 사용 루프에서 같은 모델로 이어갈 때 수정 없이 그대로 되돌려야 함.
    #[serde(rename = "thinking")]
    Thinking { thinking: String, signature: String },
    #[serde(rename = "redacted_thinking")]
    RedactedThinking { data: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct ClaudeTool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// Claude API response
#[derive(Debug, Deserialize)]
pub struct ClaudeResponse {
    pub id: String,
    pub content: Vec<ContentBlock>,
    pub model: String,
    pub stop_reason: String,
    pub usage: ClaudeUsage,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ClaudeUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

impl ClaudeUsage {
    pub fn total(&self) -> u32 { self.input_tokens + self.output_tokens }
}

/// LLM request/response
#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub system_prompt: String,
    pub messages: Vec<ClaudeMessage>,
    pub tools: Vec<ClaudeTool>,
    pub max_tokens: u32,
    pub temperature: Option<f32>,
    pub cacheable: bool,
}

#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub content: Vec<ContentBlock>,
    pub stop_reason: String,
    pub usage: ClaudeUsage,
}

/// LLM configuration
#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub max_tokens: u32,
    /// `None`이면 요청에 temperature를 넣지 않음 (최신 Claude 모델은 지정 시 400 오류)
    pub temperature: Option<f32>,
    pub timeout_secs: u64,
    pub max_retries: u32,
    pub rate_limit_rpm: u32,
    pub token_budget_daily: u64,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: "https://api.anthropic.com".into(),
            model: "claude-sonnet-5-5".into(),
            max_tokens: 4096,
            temperature: None,
            timeout_secs: 60,
            max_retries: 5,
            rate_limit_rpm: 1000,
            token_budget_daily: 1_000_000,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_sends_no_temperature() {
        // 최신 Claude 모델은 `temperature`를 거부(400: "temperature is deprecated")하므로
        // 명시적으로 설정하지 않으면 요청에 포함되지 않아야 함
        assert_eq!(LlmConfig::default().temperature, None);

        let req = ClaudeRequest {
            model: LlmConfig::default().model,
            max_tokens: 16,
            system: None,
            messages: vec![],
            tools: vec![],
            temperature: LlmConfig::default().temperature,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert!(json.get("temperature").is_none(), "temperature must be omitted: {json}");
    }

    #[test]
    fn thinking_blocks_roundtrip_unchanged() {
        let raw = r#"[{"type":"thinking","thinking":"","signature":"sig=="},
                      {"type":"redacted_thinking","data":"enc"},
                      {"type":"tool_use","id":"t1","name":"coreos_robot","input":{"command":"status"}}]"#;
        let blocks: Vec<ContentBlock> = serde_json::from_str(raw).unwrap();
        let back = serde_json::to_value(&blocks).unwrap();
        let expected: serde_json::Value = serde_json::from_str(raw).unwrap();
        assert_eq!(back, expected, "thinking blocks must be echoed back exactly");
    }

    #[test]
    fn explicit_temperature_is_sent() {
        let req = ClaudeRequest {
            model: "m".into(),
            max_tokens: 16,
            system: None,
            messages: vec![],
            tools: vec![],
            temperature: Some(0.2),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert!((json["temperature"].as_f64().unwrap() - 0.2).abs() < 1e-6);
    }
}
