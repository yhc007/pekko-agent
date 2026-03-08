/// Re-export pekko_actor's production-grade CircuitBreaker.
///
/// Previous hand-rolled implementation replaced with rust-pekko's version which
/// provides:
/// - Three-state FSM  (Closed → Open → HalfOpen → Closed)
/// - Async `call()` / `call_with_fallback()` helpers
/// - Exponential back-off on reset timeout
/// - Per-instance statistics and state-change listeners
/// - Clone-shared-state (`Arc` inside) so it is cheap to share across tasks
pub use pekko_actor::circuit_breaker::{
    CircuitBreaker,
    CircuitBreakerBuilder,
    CircuitBreakerError,
    CircuitBreakerState,
    CircuitBreakerStats,
};
