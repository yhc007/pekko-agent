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

#[derive(Clone, Debug)]
pub struct SagaExecution {
    pub execution_id: Uuid,
    pub saga: SagaDefinition,
    pub completed_steps: Vec<usize>,
    pub status: SagaStatus,
}

#[derive(Clone, Debug, PartialEq)]
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
