use crate::{ClaudeClient, LlmConfig, LlmRequest, LlmResponse, CircuitBreaker};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum LlmError {
    #[error("Circuit breaker is open")]
    CircuitOpen,
    #[error("Rate limited")]
    RateLimited,
    #[error("Token budget exceeded")]
    TokenBudgetExceeded,
    #[error("API error: status={status}, body={body}")]
    ApiError { status: u16, body: String },
    #[error("Network error: {0}")]
    NetworkError(String),
    #[error("Parse error: {0}")]
    ParseError(String),
    #[error("Timeout")]
    Timeout,
}

/// LLM Gateway - Central gateway for Claude API access
pub struct LlmGateway {
    client: ClaudeClient,
    #[allow(dead_code)]
    config: LlmConfig,
    token_budget: Arc<AtomicU64>,
    circuit_breaker: CircuitBreaker,
    request_count: AtomicU64,
}

impl LlmGateway {
    pub fn new(config: LlmConfig) -> Self {
        let budget = config.token_budget_daily;
        Self {
            client: ClaudeClient::new(config.clone()),
            config,
            token_budget: Arc::new(AtomicU64::new(budget)),
            circuit_breaker: CircuitBreaker::new(5, 3, Duration::from_secs(60)),
            request_count: AtomicU64::new(0),
        }
    }

    pub async fn call(&mut self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        if !self.circuit_breaker.try_acquire() {
            return Err(LlmError::CircuitOpen);
        }

        let remaining = self.token_budget.load(Ordering::Relaxed);
        if remaining < 1000 {
            return Err(LlmError::TokenBudgetExceeded);
        }

        match self.client.send_message(&request).await {
            Ok(response) => {
                self.circuit_breaker.record_success();
                self.token_budget.fetch_sub(
                    response.usage.total() as u64,
                    Ordering::Relaxed,
                );
                self.request_count.fetch_add(1, Ordering::Relaxed);
                Ok(response)
            }
            Err(e) => {
                self.circuit_breaker.record_failure();
                Err(match e {
                    crate::client::ClientError::ApiError { status, body } => {
                        LlmError::ApiError { status, body }
                    }
                    crate::client::ClientError::NetworkError(msg) => {
                        LlmError::NetworkError(msg)
                    }
                    crate::client::ClientError::ParseError(msg) => {
                        LlmError::ParseError(msg)
                    }
                    crate::client::ClientError::Timeout => LlmError::Timeout,
                })
            }
        }
    }

    pub fn remaining_budget(&self) -> u64 {
        self.token_budget.load(Ordering::Relaxed)
    }

    pub fn total_requests(&self) -> u64 {
        self.request_count.load(Ordering::Relaxed)
    }
}
