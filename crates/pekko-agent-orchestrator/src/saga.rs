use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::collections::HashMap;
use tracing::{info, warn};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SagaDefinition {
    pub saga_id: Uuid,
    pub name: String,
    pub steps: Vec<SagaStep>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SagaStep {
    pub step_name: String,
    pub agent_type: String,
    pub action: String,
    pub compensation_action: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SagaExecution {
    pub execution_id: Uuid,
    pub saga: SagaDefinition,
    pub completed_steps: Vec<usize>,
    pub status: SagaStatus,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SagaStatus {
    Running,
    Completed,
    Compensating { failed_at: usize },
    CompensationCompleted,
    Failed { error: String },
}

pub struct SagaManager {
    sagas: HashMap<Uuid, SagaDefinition>,
    executions: HashMap<Uuid, SagaExecution>,
}

impl SagaManager {
    pub fn new() -> Self {
        Self {
            sagas: HashMap::new(),
            executions: HashMap::new(),
        }
    }

    pub fn register_saga(&mut self, saga: SagaDefinition) {
        info!(saga_id = %saga.saga_id, name = %saga.name, "Saga registered");
        self.sagas.insert(saga.saga_id, saga);
    }

    pub fn start_execution(&mut self, saga_id: &Uuid) -> Option<Uuid> {
        let saga = self.sagas.get(saga_id)?.clone();
        let execution_id = Uuid::new_v4();
        let execution = SagaExecution {
            execution_id,
            saga,
            completed_steps: Vec::new(),
            status: SagaStatus::Running,
        };
        info!(execution_id = %execution_id, saga_id = %saga_id, "Saga execution started");
        self.executions.insert(execution_id, execution);
        Some(execution_id)
    }

    pub fn complete_step(&mut self, execution_id: &Uuid, step_index: usize) {
        if let Some(exec) = self.executions.get_mut(execution_id) {
            exec.completed_steps.push(step_index);
            if exec.completed_steps.len() == exec.saga.steps.len() {
                exec.status = SagaStatus::Completed;
                info!(execution_id = %execution_id, "Saga completed");
            }
        }
    }

    pub fn fail_step(&mut self, execution_id: &Uuid, step_index: usize) {
        if let Some(exec) = self.executions.get_mut(execution_id) {
            exec.status = SagaStatus::Compensating { failed_at: step_index };
            warn!(execution_id = %execution_id, step = step_index, "Saga step failed, compensating");
        }
    }

    pub fn get_compensation_steps(&self, execution_id: &Uuid) -> Vec<&SagaStep> {
        self.executions.get(execution_id)
            .map(|exec| {
                exec.completed_steps.iter()
                    .rev()
                    .filter_map(|&idx| exec.saga.steps.get(idx))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn get_execution(&self, execution_id: &Uuid) -> Option<&SagaExecution> {
        self.executions.get(execution_id)
    }
}

// ─── pekko_persistence::PersistentActor impl ─────────────────────────────────
//
// SagaManager is now a persistent actor: every step-completion and
// failure is journalled, so a crashed orchestrator can replay the saga
// execution log and resume compensating transactions from where it left off.

use async_trait::async_trait;
use pekko_actor::{Actor, ActorContext};
use pekko_persistence::{PersistentActor, PersistentContext};

// ── Actor messages ────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum SagaMessage {
    Register(SagaDefinition),
    StartExecution(Uuid),
    CompleteStep { execution_id: Uuid, step_index: usize },
    FailStep     { execution_id: Uuid, step_index: usize },
}

// ── pekko_actor::Actor ────────────────────────────────────────────────────────

#[async_trait]
impl Actor for SagaManager {
    type Message = SagaMessage;

    async fn receive(&mut self, msg: Self::Message, _ctx: &mut ActorContext<Self>) {
        match msg {
            SagaMessage::Register(saga)         => { self.register_saga(saga); }
            SagaMessage::StartExecution(id)     => { self.start_execution(&id); }
            SagaMessage::CompleteStep { execution_id, step_index } => {
                self.complete_step(&execution_id, step_index);
            }
            SagaMessage::FailStep { execution_id, step_index } => {
                self.fail_step(&execution_id, step_index);
            }
        }
    }
}

// ── pekko_persistence::PersistentActor ───────────────────────────────────────

/// Journalled saga events (what gets persisted to the Journal).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum SagaJournalEvent {
    SagaRegistered { saga: SagaDefinition },
    ExecutionStarted { execution_id: Uuid, saga_id: Uuid },
    StepCompleted { execution_id: Uuid, step_index: usize },
    StepFailed    { execution_id: Uuid, step_index: usize },
}

/// Snapshot = full in-memory state (fast recovery).
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct SagaManagerSnapshot {
    pub sagas:      std::collections::HashMap<Uuid, SagaDefinition>,
    pub executions: std::collections::HashMap<Uuid, SagaExecution>,
}

#[async_trait]
impl PersistentActor for SagaManager {
    type Event    = SagaJournalEvent;
    type State    = SagaManagerSnapshot;
    type Snapshot = SagaManagerSnapshot;

    fn persistence_id(&self) -> String {
        "saga-manager-singleton".to_string()
    }

    async fn receive_recover(
        &mut self,
        event: Self::Event,
        _ctx: &mut PersistentContext<Self>,
    ) {
        // Replay journal events to restore in-memory state.
        match event {
            SagaJournalEvent::SagaRegistered { saga } => {
                self.sagas.insert(saga.saga_id, saga);
            }
            SagaJournalEvent::ExecutionStarted { execution_id, saga_id } => {
                if let Some(saga) = self.sagas.get(&saga_id).cloned() {
                    self.executions.insert(execution_id, SagaExecution {
                        execution_id,
                        saga,
                        completed_steps: vec![],
                        status: SagaStatus::Running,
                    });
                }
            }
            SagaJournalEvent::StepCompleted { execution_id, step_index } => {
                self.complete_step(&execution_id, step_index);
            }
            SagaJournalEvent::StepFailed { execution_id, step_index } => {
                self.fail_step(&execution_id, step_index);
            }
        }
    }

    async fn receive_command(
        &mut self,
        msg: Self::Message,
        ctx: &mut PersistentContext<Self>,
    ) {
        match msg {
            SagaMessage::Register(saga) => {
                let event = SagaJournalEvent::SagaRegistered { saga: saga.clone() };
                let _ = ctx.persist(event).await;
                self.register_saga(saga);
            }
            SagaMessage::StartExecution(saga_id) => {
                if let Some(execution_id) = self.start_execution(&saga_id) {
                    let event = SagaJournalEvent::ExecutionStarted { execution_id, saga_id };
                    let _ = ctx.persist(event).await;
                }
            }
            SagaMessage::CompleteStep { execution_id, step_index } => {
                let event = SagaJournalEvent::StepCompleted { execution_id, step_index };
                let _ = ctx.persist(event).await;
                self.complete_step(&execution_id, step_index);
            }
            SagaMessage::FailStep { execution_id, step_index } => {
                let event = SagaJournalEvent::StepFailed { execution_id, step_index };
                let _ = ctx.persist(event).await;
                self.fail_step(&execution_id, step_index);
            }
        }
    }

    fn apply_event(&mut self, _event: &Self::Event) -> Self::State {
        SagaManagerSnapshot {
            sagas:      self.sagas.clone(),
            executions: self.executions.clone(),
        }
    }

    fn create_snapshot(&self) -> Self::Snapshot {
        SagaManagerSnapshot {
            sagas:      self.sagas.clone(),
            executions: self.executions.clone(),
        }
    }

    fn apply_snapshot(&mut self, snapshot: Self::Snapshot) {
        self.sagas      = snapshot.sagas;
        self.executions = snapshot.executions;
    }
}
