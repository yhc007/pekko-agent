use crate::types::*;
use reqwest::Client;
use std::time::Duration;
use tracing::{info, warn};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ClientError {
    #[error("API error: status={status}, body={body}")]
    ApiError { status: u16, body: String },
    #[error("Network error: {0}")]
    NetworkError(String),
    #[error("Parse error: {0}")]
    ParseError(String),
    #[error("Timeout")]
    Timeout,
}

/// Claude API Client
pub struct ClaudeClient {
    client: Client,
    config: LlmConfig,
}

impl ClaudeClient {
    pub fn new(config: LlmConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .build()
            .expect("Failed to build HTTP client");
        Self { client, config }
    }

    pub async fn send_message(
        &self,
        request: &LlmRequest,
    ) -> Result<LlmResponse, ClientError> {
        let claude_req = ClaudeRequest {
            model: self.config.model.clone(),
            max_tokens: request.max_tokens,
            system: Some(request.system_prompt.clone()),
            messages: request.messages.clone(),
            tools: request.tools.clone(),
            temperature: request.temperature,
        };

        let mut retries = 0;
        loop {
            let resp = self
                .client
                .post(format!("{}/v1/messages", self.config.base_url))
                .header("x-api-key", &self.config.api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&claude_req)
                .send()
                .await;

            match resp {
                Ok(response) if response.status().is_success() => {
                    let body: ClaudeResponse = response.json().await
                        .map_err(|e| ClientError::ParseError(e.to_string()))?;
                    
                    info!(
                        model = %body.model,
                        input_tokens = body.usage.input_tokens,
                        output_tokens = body.usage.output_tokens,
                        "Claude API call succeeded"
                    );

                    return Ok(LlmResponse {
                        content: body.content,
                        stop_reason: body.stop_reason,
                        usage: body.usage,
                    });
                }
                Ok(response) if response.status().as_u16() == 429 => {
                    retries += 1;
                    if retries > self.config.max_retries {
                        return Err(ClientError::NetworkError("Rate limit exceeded".into()));
                    }
                    warn!(retry = retries, "Rate limited, retrying...");
                    tokio::time::sleep(Duration::from_secs(2u64.pow(retries))).await;
                }
                Ok(response) => {
                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();
                    return Err(ClientError::ApiError { status: status.as_u16(), body });
                }
                Err(e) if retries < self.config.max_retries => {
                    retries += 1;
                    warn!(retry = retries, error = %e, "Request failed, retrying...");
                    tokio::time::sleep(Duration::from_secs(2u64.pow(retries))).await;
                }
                Err(e) => {
                    return Err(ClientError::NetworkError(e.to_string()));
                }
            }
        }
    }
}
