pub mod agent;
pub mod message;
pub mod state;
pub mod tool;
pub mod memory;
pub mod error;

pub use agent::*;
pub use message::*;
pub use state::*;
pub use tool::*;
pub use memory::*;
pub use error::*;

// ── rust-pekko re-exports ────────────────────────────────────────────────────
// Downstream crates (EHS services, orchestrator, …) can simply use
// `pekko_agent_core::{ActorSystem, Actor, ActorRef, Props, …}`
// without adding a direct dependency on pekko-actor or pekko-persistence.
//
// LOCAL DEV PATCH (HR-AI workspace, 2026-05-20): 아래 8개 타입(FsmStateMachine,
// StateMachineBuilder, TransitionResult, CircuitBreaker*, Scheduler)은 현재
// 체크아웃된 rust-pekko/pekko-actor에 정의되어 있지 않아 컴파일이 실패한다.
// pekko-agent-core 자체는 이 타입들을 내부적으로 사용하지 않으므로(순수 convenience
// re-export) HR-AI 통합을 위해 임시로 주석 처리. pekko-actor가 FSM/CircuitBreaker/
// Scheduler를 제공하게 되면 복원할 것. (pekko-agent-llm / orchestrator는 이 타입들을
// 직접 쓰므로 그 crate를 빌드할 때는 pekko-actor 보강이 선행되어야 한다.)
pub use pekko_actor::{Actor, ActorContext, ActorRef, ActorSystem, Props};
// pub use pekko_actor::{
//     FsmStateMachine, StateMachineBuilder, TransitionResult,
//     CircuitBreaker, CircuitBreakerBuilder, CircuitBreakerError, CircuitBreakerState,
//     Scheduler,
// };
