use crate::{ClaudeClient, LlmConfig, LlmRequest, LlmResponse};
use crate::circuit_breaker::{CircuitBreaker, CircuitBreakerError};
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

/// LLM Gateway — Central gateway for Claude API access.
///
/// Uses the production-grade `pekko_actor::CircuitBreaker` (builder pattern,
/// async `call()`, exponential back-off, statistics) in place of the previous
/// hand-rolled boolean-state breaker.
pub struct LlmGateway {
    client: Arc<ClaudeClient>,
    #[allow(dead_code)]
    config: LlmConfig,
    token_budget: Arc<AtomicU64>,
    /// Shared circuit breaker — cheap to clone because the inner state is Arc'd.
    circuit_breaker: CircuitBreaker,
    request_count: AtomicU64,
}

impl LlmGateway {
    pub fn new(config: LlmConfig) -> Self {
        let budget = config.token_budget_daily;

        // Build the circuit breaker with pekko_actor's builder API.
        let circuit_breaker = CircuitBreaker::new()
            .max_failures(5)
            .call_timeout(Duration::from_secs(30))
            .reset_timeout(Duration::from_secs(60))
            .max_half_open_calls(1)
            .exponential_backoff(1.5)
            .build();

        Self {
            client: Arc::new(ClaudeClient::new(config.clone())),
            config,
            token_budget: Arc::new(AtomicU64::new(budget)),
            circuit_breaker,
            request_count: AtomicU64::new(0),
        }
    }

    pub async fn call(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        // Token budget guard (fast path before hitting the circuit breaker).
        let remaining = self.token_budget.load(Ordering::Relaxed);
        if remaining < 1000 {
            return Err(LlmError::TokenBudgetExceeded);
        }

        let client = self.client.clone();
        let token_budget = self.token_budget.clone();
        let request_count = &self.request_count;

        // Delegate to pekko_actor CircuitBreaker.call(); it handles
        // Open/HalfOpen/Closed transitions and the call timeout internally.
        let result = self.circuit_breaker
            .call(|| async move {
                client.send_message(&request).await
                    .map_err(|e| e) // keep ClientError as-is inside the closure
            })
            .await;

        match result {
            Ok(response) => {
                token_budget.fetch_sub(response.usage.total() as u64, Ordering::Relaxed);
                request_count.fetch_add(1, Ordering::Relaxed);
                Ok(response)
            }
            Err(CircuitBreakerError::Open) => Err(LlmError::CircuitOpen),
            Err(CircuitBreakerError::Timeout) => Err(LlmError::Timeout),
            Err(CircuitBreakerError::CallFailed(e)) => {
                use crate::client::ClientError;
                Err(match e {
                    ClientError::ApiError { status, body } => LlmError::ApiError { status, body },
                    ClientError::NetworkError(msg) => LlmError::NetworkError(msg),
                    ClientError::ParseError(msg) => LlmError::ParseError(msg),
                    ClientError::Timeout => LlmError::Timeout,
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

    /// Access circuit-breaker statistics exposed by pekko_actor.
    pub fn circuit_breaker_stats(&self) -> pekko_actor::CircuitBreakerStats {
        self.circuit_breaker.stats()
    }
}
