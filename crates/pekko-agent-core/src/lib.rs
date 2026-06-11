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
// agent::AgentMessage (Query/Execute/Respond)와 message::AgentMessage가 충돌하므로
// Actor 메시지 타입인 agent::AgentMessage를 명시적으로 재-export해 우선권을 부여한다.
pub use agent::AgentMessage;

// ── rust-pekko re-exports ────────────────────────────────────────────────────
// Downstream crates (EHS services, orchestrator, …) can simply use
// `pekko_agent_core::{ActorSystem, Actor, ActorRef, Props, …}`
// without adding a direct dependency on pekko-actor or pekko-persistence.
//
// Scheduler는 pekko-actor에 미구현 — 추가되면 복원.
// pub use pekko_actor::Scheduler;
pub use pekko_actor::{
    Actor, ActorContext, ActorRef, ActorSystem, Props,
    CircuitBreaker, CircuitBreakerBuilder, CircuitBreakerError, CircuitBreakerState,
    FsmStateMachine, StateMachineBuilder, TransitionResult,
};
