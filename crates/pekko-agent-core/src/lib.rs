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
// `pekko_agent_core::{ActorSystem, Actor, ActorRef, Props, FsmStateMachine, …}`
// without adding a direct dependency on pekko-actor or pekko-persistence.
pub use pekko_actor::{
    Actor, ActorContext, ActorRef, ActorSystem, Props,
    FsmStateMachine, StateMachineBuilder, TransitionResult,
    CircuitBreaker, CircuitBreakerBuilder, CircuitBreakerError, CircuitBreakerState,
    Scheduler,
};
